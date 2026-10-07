//! Inject Angular's Ivy `.d.ts` type declarations into emitted declaration
//! files for library builds.
//!
//! The compiler returns, per Angular class, the static member type
//! declarations that should live in the class's `.d.ts` body — e.g.
//! `static ɵcmp: i0.ɵɵComponentDeclaration<…>;`. Those members are what
//! Angular's template type-checker reads from a pre-compiled library, and
//! they mirror what ngtsc's `IvyDeclarationDtsTransform` would have
//! written.
//!
//! Vite/Rolldown don't emit `.d.ts` themselves — a separate declaration
//! generator (rolldown-plugin-dts, vite-plugin-dts, tsdown, `tsc`) produces
//! the base declarations. This is a post-processing pass that splices the
//! Angular members into those already-generated `.d.ts`, and emits the
//! namespace imports the members reference.
//!
//! ngtsc never reuses an existing namespace import in declaration emit —
//! its `ImportManager` (`presetImportManagerForceNamespaceImports`) mints
//! `i0`, `i0_1`, … deduped against the ORIGINAL source file's identifiers.
//! The compiler mirrors that: each declaration's members carry the alias
//! the per-source-file registry picked, and `namespaceImports` records
//! which module each alias stands for. A bundled `.d.ts` merges
//! declarations from many source files whose aliases can disagree (`i1` →
//! `./dep` in one, `./other` in another), so this pass canonicalizes: one
//! alias per module identity, collision-free against every identifier in
//! the emitted file.

use rustc_hash::{FxHashMap, FxHashSet};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ClassBody, ClassElement, Declaration, Expression, ImportDeclarationSpecifier, Program,
    PropertyKey, Statement, TSNamespaceDeclarationBody, TSQualifiedName, TSTypeName,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};

use crate::r3::{CORE, CORE_ALIAS};

/// One class's `.d.ts` static member declarations.
pub struct DtsInjectDeclaration {
    /// The class the members belong to.
    pub class_name: String,
    /// Newline-separated `static …;` member declarations.
    /// Namespace-qualified type names (`i0.ɵɵComponentDeclaration`,
    /// `typeof i1.SomeDirective`) use the alias the compiler picked for the
    /// member's module in that source file; `namespace_imports` maps each
    /// such alias to its module specifier.
    pub members: String,
    /// alias → module specifier for every namespace the members reference:
    /// `i0` → `"@angular/core"` for `i0.ɵɵX`/`i0.Signal`, `i1` → `"./dep"`
    /// for `typeof i1.SomeDirective` host-directive references, imported
    /// ctor-dep types, and `ngAcceptInputType_*` transform types alike.
    /// Relative specifiers (`"./dep"`) are resolved against `source_file`
    /// for identity (so the same specifier under different directories is
    /// two modules) and emitted verbatim — structure-preserving
    /// declaration emit places `dist/a/foo.d.ts` next to `dist/a/dep.d.ts`,
    /// same as the source.
    pub namespace_imports: FxHashMap<String, String>,
    /// The source module the declaration was compiled from (the
    /// transform's module id). Used to resolve relative specifiers in
    /// `namespace_imports` so two files in different directories that each
    /// import `"./dep"` canonicalize to different aliases instead of
    /// silently sharing one.
    pub source_file: Option<String>,
}

/// One namespace-alias head found in a member body.
struct MemberHead {
    /// `(start, end)` of the head inside the member's own text.
    start: usize,
    end: usize,
    /// The head's alias as compiled (what `member[start..end]` reads).
    alias: String,
    /// The module specifier the alias refers to — emitted verbatim,
    /// matching ngtsc and structure-preserving declaration emit.
    module: String,
    /// The canonical identity for deduping — the specifier resolved against
    /// the declaration's source file, so `"./dep"` under different
    /// directories is two modules, not one.
    resolved: String,
}

/// `i0`, `i0_1`, `i0_2`, … — the first name not colliding with `names`.
/// Matches `check_unique_identifier_name` in ngtsc's ImportManager.
fn uniquify_identifier(base: &str, names: &FxHashSet<String>) -> String {
    if !names.contains(base) {
        return base.to_string();
    }
    let mut counter = 1;
    while names.contains(&format!("{base}_{counter}")) {
        counter += 1;
    }
    format!("{base}_{counter}")
}

/// The alias → specifier map of every `import * as ns` the file already
/// has.
fn namespace_imports_of(program: &Program<'_>) -> FxHashMap<String, String> {
    let mut aliases = FxHashMap::default();
    for stmt in &program.body {
        let Statement::ImportDeclaration(decl) = stmt else { continue };
        let Some(specifiers) = &decl.specifiers else { continue };
        for spec in specifiers {
            if let ImportDeclarationSpecifier::ImportNamespaceSpecifier(ns) = spec {
                aliases.insert(ns.local.name.to_string(), decl.source.value.to_string());
            }
        }
    }
    aliases
}

/// Every identifier name in the file — the same set the JS `Identifier`
/// visitor collected (`IdentifierName`, `IdentifierReference`,
/// `BindingIdentifier`, `LabelIdentifier`).
struct IdentifierCollector<'a> {
    names: &'a mut FxHashSet<String>,
}

impl<'a> Visit<'a> for IdentifierCollector<'_> {
    fn visit_identifier_name(&mut self, it: &oxc_ast::ast::IdentifierName<'a>) {
        self.names.insert(it.name.to_string());
        walk::walk_identifier_name(self, it);
    }

    fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
        self.names.insert(it.name.to_string());
        walk::walk_identifier_reference(self, it);
    }

    fn visit_binding_identifier(&mut self, it: &oxc_ast::ast::BindingIdentifier<'a>) {
        self.names.insert(it.name.to_string());
        walk::walk_binding_identifier(self, it);
    }

    fn visit_label_identifier(&mut self, it: &oxc_ast::ast::LabelIdentifier<'a>) {
        self.names.insert(it.name.to_string());
        walk::walk_label_identifier(self, it);
    }
}

fn collect_identifiers(program: &Program<'_>) -> FxHashSet<String> {
    let mut names = FxHashSet::default();
    let mut collector = IdentifierCollector { names: &mut names };
    collector.visit_program(program);
    names
}

/// The `ClassBody` of the class named `class_name`, searching through
/// export wrappers and ambient module blocks (the latter covers
/// `declare module` declarations a bundler may wrap library classes in).
fn find_class_body<'a, 'b>(
    statements: &'b [Statement<'a>],
    class_name: &str,
) -> Option<&'b ClassBody<'a>> {
    fn from_namespace<'a, 'b>(
        ns: &'b oxc_ast::ast::TSNamespaceDeclaration<'a>,
        class_name: &str,
    ) -> Option<&'b ClassBody<'a>> {
        match &ns.body {
            TSNamespaceDeclarationBody::TSModuleBlock(block) => {
                find_class_body(&block.body, class_name)
            }
            TSNamespaceDeclarationBody::TSNamespaceDeclaration(inner) => {
                from_namespace(inner, class_name)
            }
        }
    }

    fn from_declaration<'a, 'b>(
        decl: &'b Declaration<'a>,
        class_name: &str,
    ) -> Option<&'b ClassBody<'a>> {
        match decl {
            Declaration::ClassDeclaration(class) => {
                if class.id.as_ref().map(|id| id.name.as_str()) == Some(class_name) {
                    Some(&*class.body)
                } else {
                    None
                }
            }
            // `declare module "pkg" { export class … }`
            Declaration::TSExternalModuleDeclaration(module) => {
                module.body.as_deref().and_then(|block| find_class_body(&block.body, class_name))
            }
            Declaration::TSNamespaceDeclaration(ns) => from_namespace(ns, class_name),
            Declaration::TSGlobalDeclaration(global) => {
                find_class_body(&global.body.body, class_name)
            }
            _ => None,
        }
    }

    for stmt in statements {
        let found = match stmt {
            // `export declare class` — oxc splits export-with-declaration
            // into `ExportDeclaration`, while `export { a }` is
            // `ExportNamedDeclaration` (which never wraps a declaration).
            Statement::ExportDeclaration(export) => {
                from_declaration(&export.declaration, class_name)
            }
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class)
                    if class.id.as_ref().map(|id| id.name.as_str()) == Some(class_name) =>
                {
                    Some(&*class.body)
                }
                _ => None,
            },
            other => other.as_declaration().and_then(|decl| from_declaration(decl, class_name)),
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The member's name as written, if it is a non-computed static or string
/// literal key.
fn property_key_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.to_string()),
        _ => match key.as_expression() {
            Some(Expression::StringLiteral(lit)) => Some(lit.value.to_string()),
            Some(Expression::NumericLiteral(lit)) => Some(lit.value.to_string()),
            _ => None,
        },
    }
}

fn class_element_span(element: &ClassElement<'_>) -> Span {
    match element {
        ClassElement::StaticBlock(el) => el.span,
        ClassElement::MethodDefinition(el) => el.span,
        ClassElement::PropertyDefinition(el) => el.span,
        ClassElement::AccessorProperty(el) => el.span,
        ClassElement::TSIndexSignature(el) => el.span,
    }
}

fn class_element_parts<'a, 'b>(
    element: &'b ClassElement<'a>,
) -> (bool, bool, Option<&'b PropertyKey<'a>>) {
    match element {
        ClassElement::StaticBlock(_) | ClassElement::TSIndexSignature(_) => (false, false, None),
        ClassElement::MethodDefinition(el) => (el.r#static, el.computed, Some(&el.key)),
        ClassElement::PropertyDefinition(el) => (el.r#static, el.computed, Some(&el.key)),
        ClassElement::AccessorProperty(el) => (el.r#static, el.computed, Some(&el.key)),
    }
}

/// Every non-computed STATIC member name in `body`, as written. Instance
/// members don't count: TypeScript permits `ɵfac` and `static ɵfac` to
/// coexist, and only a previously generated static is idempotent.
fn existing_member_names(body: &ClassBody<'_>) -> FxHashSet<String> {
    let mut names = FxHashSet::default();
    for element in &body.body {
        let (is_static, computed, key) = class_element_parts(element);
        if !is_static || computed {
            continue;
        }
        if let Some(Some(name)) = key.map(property_key_name) {
            names.insert(name);
        }
    }
    names
}

/// One parsed member: its name (if a static identifier/literal key), its
/// exact text slice, and its namespace heads.
struct ParsedMember {
    name: Option<String>,
    text: String,
    heads: Vec<MemberHead>,
}

/// The `TSQualifiedName` heads inside `members`, collected in one pass over
/// the wrapped program (mirroring the JS Visitor's always-descend
/// semantics: children are walked even after a head is recorded).
struct HeadCollector<'ns, 'r> {
    namespace_imports: &'ns FxHashMap<String, String>,
    resolve: &'r dyn Fn(&str) -> String,
    offset: usize,
    collected: Vec<(usize, String, String, String)>,
}

/// `i0`, `i0_1`, `i0_2`, … — the aliases the namespace registry prefers
/// for `@angular/core` (`CORE_ALIAS` uniquified). When `namespace_imports`
/// is absent these are the only unmapped heads safe to infer as core,
/// preserving the pre-canonicalization behavior of always emitting the
/// `i0` import. `i1`+ aliases can stand for any module and stay untouched.
fn is_core_alias_convention(alias: &str) -> bool {
    let uniquified_prefix = format!("{CORE_ALIAS}_");
    alias == CORE_ALIAS
        || alias
            .strip_prefix(uniquified_prefix.as_str())
            .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

impl<'a> Visit<'a> for HeadCollector<'_, '_> {
    fn visit_ts_qualified_name(&mut self, node: &TSQualifiedName<'a>) {
        if let TSTypeName::IdentifierReference(left) = &node.left {
            let alias = left.name.to_string();
            let module = self.namespace_imports.get(&alias);
            let unmapped_core =
                self.namespace_imports.is_empty() && is_core_alias_convention(&alias);
            if module.is_some() || node.right.name.starts_with('ɵ') || unmapped_core {
                let specifier = module.cloned().unwrap_or_else(|| CORE.to_string());
                self.collected.push((
                    left.span.start as usize - self.offset,
                    alias,
                    specifier.clone(),
                    (self.resolve)(&specifier),
                ));
            }
        }
        // Always descend: `a.b.c` also visits `a.b`, whose head is `a` —
        // dedup happens implicitly because only the innermost node has an
        // identifier `left`.
        walk::walk_ts_qualified_name(self, node);
    }
}

/// Parse `members` (wrapped in a dummy `declare class`) into per-member
/// entries. Each head is the leftmost identifier of a `TSQualifiedName`
/// (`i0.ɵɵX` and `typeof i1.SomeDirective` alike — a nested `a.b.c` chain
/// is visited as its inner `a.b` too, so the head is always found).
///
/// Returns `None` when the member text doesn't parse; the caller skips the
/// declaration rather than splice members whose aliases might be wrong.
fn parse_members(
    members: &str,
    namespace_imports: &FxHashMap<String, String>,
    resolve: &dyn Fn(&str) -> String,
) -> Option<Vec<ParsedMember>> {
    const PREFIX: &str = "declare class X {\n";
    let wrapped = format!("{PREFIX}{members}\n}}");

    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, &wrapped, SourceType::d_ts()).parse();
    if !ret.diagnostics.is_empty() {
        return None;
    }
    let Some(Statement::ClassDeclaration(cls)) = ret.program.body.first() else {
        return Some(Vec::new());
    };

    let mut collector =
        HeadCollector { namespace_imports, resolve, offset: PREFIX.len(), collected: Vec::new() };
    collector.visit_program(&ret.program);
    let collected = collector.collected;

    let parsed: Vec<ParsedMember> = cls
        .body
        .body
        .iter()
        .map(|element| {
            // Spans are relative to `wrapped`; shift them back into `members`.
            let span = class_element_span(element);
            let start = span.start as usize - PREFIX.len();
            let end = span.end as usize - PREFIX.len();
            let (_, computed, key) = class_element_parts(element);
            let name = if computed { None } else { key.and_then(property_key_name) };
            let heads: Vec<MemberHead> = collected
                .iter()
                .filter(|(pos, ..)| *pos >= start && *pos < end)
                .map(|(pos, alias, module, resolved)| MemberHead {
                    start: pos - start,
                    end: pos - start + alias.len(),
                    alias: alias.clone(),
                    module: module.clone(),
                    resolved: resolved.clone(),
                })
                .collect();
            ParsedMember {
                name,
                text: members.get(start..end).unwrap_or_default().to_string(),
                heads,
            }
        })
        .collect();
    Some(parsed)
}

/// The position where a new import can be spliced: just past `position`
/// and any comments that begin on the same line (trailing comments,
/// multiline included). Code sharing the import's line
/// (`import …; export declare class X {`) must stay below the new import,
/// so the scan never advances past non-comment syntax.
fn import_insert_position(
    source: &str,
    position: usize,
    comment_spans: &[(usize, usize)],
) -> usize {
    let bytes = source.as_bytes();
    let mut cursor = position;
    loop {
        let line_end =
            bytes[cursor..].iter().position(|&b| b == b'\n').map_or(source.len(), |i| cursor + i);
        // A comment beginning on this line at-or-after `cursor` is a
        // trailing comment of the import; keep it on the import's line.
        let next = comment_spans
            .iter()
            .filter(|&&(start, _)| start >= cursor && start < line_end)
            .map(|&(_, end)| end)
            .min();
        match next {
            Some(comment_end) => cursor = comment_end,
            None => return cursor,
        }
    }
}

/// Resolve `dir + specifier` the way Node would see it — collapsing `..`
/// and `.` segments — so `"./dep"` under different directories
/// canonicalizes to different identities while `../dep` from a nested file
/// resolves back to the same one. Purely textual; no filesystem access.
fn resolve_path(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            out.pop();
        } else {
            out.push(segment);
        }
    }
    out.join("/")
}

/// Splice each declaration's static members into the matching class body.
///
/// Emits the namespace imports the injected members reference — one
/// `import * as ns from "module"` per module identity, canonicalized
/// across all injected declarations.
///
/// Like ngtsc's declaration ImportManager, an existing `import * as ns` is
/// reused only when it already binds the same alias and specifier — never
/// for a different one, and never to borrow the user's `@angular/core`
/// import: the members' compiled alias is kept when free, otherwise
/// uniquified (`i0_1`, …).
///
/// The pass is idempotent: a member whose STATIC name already appears in
/// the target class is skipped — structurally, on the member name, so a
/// second pass that canonicalizes to a different alias can't inject the
/// same member twice. A declaration whose class isn't found, whose members
/// don't parse, or a file the parser rejects is silently skipped.
pub fn inject_dts_declarations(source: &str, declarations: &[DtsInjectDeclaration]) -> String {
    if declarations.is_empty() {
        return source.to_string();
    }

    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, SourceType::d_ts()).parse();
    if !ret.diagnostics.is_empty() {
        return source.to_string();
    }
    let program = &ret.program;

    let mut file_names = collect_identifiers(program);
    let existing_imports = namespace_imports_of(program);

    // Identity resolution is relative to each declaration's source module —
    // the same "./dep" in two directories is two modules. Emitted
    // specifiers stay verbatim, matching ngtsc and structure-preserving
    // declaration emit (`src/a/foo.ts` → `dist/a/foo.d.ts`, where "./dep"
    // reaches `dist/a/dep.d.ts`); there is no dts-generator source→output
    // mapping to rebase against, so under fully-bundled declaration output
    // two same-named relative modules remain a documented limitation.
    let resolve_for = |source_file: Option<&str>| -> Box<dyn Fn(&str) -> String> {
        match source_file {
            None => Box::new(|specifier: &str| specifier.to_string()),
            Some(file) => {
                let dir_end = file.rfind(['/', '\\']).map_or(0, |i| i + 1);
                let dir = file[..dir_end].to_string();
                Box::new(move |specifier: &str| {
                    if specifier.starts_with("./") || specifier.starts_with("../") {
                        resolve_path(&format!("{dir}{specifier}"))
                    } else {
                        specifier.to_string()
                    }
                })
            }
        }
    };

    // Canonical module identity → [rawSpecifier, alias] for this file.
    let mut canonical: FxHashMap<String, (String, String)> = FxHashMap::default();
    let mut canonical_alias = |head: &MemberHead| -> String {
        if let Some((_, alias)) = canonical.get(&head.resolved) {
            return alias.clone();
        }
        let preferred = if head.module == CORE { CORE_ALIAS } else { head.alias.as_str() };
        // Reuse an existing import only when it binds this alias AND
        // specifier — same module under the same name (which is exactly
        // what an idempotent re-run sees on its second pass), never a
        // different module.
        let alias = match existing_imports.get(preferred) {
            Some(existing) if existing == &head.module => preferred.to_string(),
            _ => uniquify_identifier(preferred, &file_names),
        };
        canonical.insert(head.resolved.clone(), (head.module.clone(), alias.clone()));
        file_names.insert(alias.clone());
        alias
    };

    let mut splices: Vec<(usize, usize, String)> = Vec::new();

    for declaration in declarations {
        let resolve = resolve_for(declaration.source_file.as_deref());
        let Some(members) =
            parse_members(&declaration.members, &declaration.namespace_imports, &*resolve)
        else {
            continue;
        };
        if members.is_empty() {
            continue;
        }

        let Some(body) = find_class_body(&program.body, &declaration.class_name) else {
            continue;
        };
        let existing = existing_member_names(body);

        // Per-member idempotency: skip members the class already declares.
        // Names — not normalized text — decide, so a re-run whose
        // canonical alias differs (the first pass's import now occupies
        // `i0`) doesn't inject the same `static ɵfac` a second time.
        let mut lines: Vec<String> = Vec::new();
        for member in &members {
            if let Some(name) = &member.name
                && existing.contains(name)
            {
                continue;
            }
            let mut text = member.text.clone();
            let mut heads: Vec<&MemberHead> = member.heads.iter().collect();
            heads.sort_by_key(|head| std::cmp::Reverse(head.start));
            for head in heads {
                let alias = canonical_alias(head);
                text.replace_range(head.start..head.end, &alias);
            }
            for line in text.split('\n') {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    lines.push(trimmed.to_string());
                }
            }
        }
        if lines.is_empty() {
            continue;
        }

        // `ClassBody.span.end` is just past the closing brace; inserting
        // before it appends the members, matching upstream's
        // `[...members, ...newMembers]` (declaration.ts) — a class that
        // already has members keeps them first.
        let insert_at = body.span.end as usize - 1;
        let needs_leading_nl = insert_at > 0 && source.as_bytes()[insert_at - 1] != b'\n';
        let text = format!(
            "{}{}\n",
            if needs_leading_nl { "\n" } else { "" },
            lines.iter().map(|line| format!("    {line}")).collect::<Vec<_>>().join("\n")
        );
        splices.push((insert_at, insert_at, text));
    }

    if splices.is_empty() {
        return source.to_string();
    }

    // Emit an import for every canonical module the file doesn't already
    // have, sorted by alias — the same order the compiler's own namespace
    // registry generates them in.
    let mut import_entries: Vec<(&String, &String)> = canonical
        .values()
        .filter(|(specifier, alias)| existing_imports.get(alias.as_str()) != Some(specifier))
        .map(|(specifier, alias)| (alias, specifier))
        .collect();
    import_entries.sort();
    let import_lines: Vec<String> = import_entries
        .iter()
        .map(|(alias, specifier)| format!("import * as {alias} from \"{specifier}\";"))
        .collect();

    if !import_lines.is_empty() {
        let text = import_lines.join("\n");
        let last_import_end = program.body.iter().rev().find_map(|stmt| match stmt {
            Statement::ImportDeclaration(decl) => Some(decl.span.end as usize),
            _ => None,
        });
        if let Some(end) = last_import_end {
            // After the last import and any trailing comments on its line
            // (`import x from "m"; /* keep\nme */` keeps the comment with
            // it; `import x; export class` doesn't spill inside the class).
            let comment_spans: Vec<(usize, usize)> = program
                .comments
                .iter()
                .map(|c| (c.span.start as usize, c.span.end as usize))
                .collect();
            let position = import_insert_position(source, end, &comment_spans);
            // Source's own newline at `position` separates the new import;
            // otherwise we open a line — and close ours too when code
            // follows on the same line.
            let at_newline = source.as_bytes().get(position) == Some(&b'\n');
            splices.push((
                position,
                position,
                format!(
                    "\n{text}{}",
                    if at_newline || position == source.len() { "" } else { "\n" }
                ),
            ));
        } else if let Some(first) = program.body.first() {
            // Before the first statement keeps leading comments and
            // triple-slash reference directives at the top of the file.
            let start = first.span().start as usize;
            splices.push((start, start, format!("{text}\n")));
        } else {
            splices.push((
                source.len(),
                source.len(),
                format!(
                    "{}{text}\n",
                    if source.ends_with('\n') || source.is_empty() { "" } else { "\n" }
                ),
            ));
        }
    }

    let mut output = source.to_string();
    splices.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start, end, text) in splices {
        output.replace_range(start..end, &text);
    }
    output
}
