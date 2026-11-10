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
    /// The class the members belong to; dotted (`NS.App`) for a class of a
    /// `namespace`.
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
            // A source with a lone surrogate (`'\uD800'`) never matches a
            // generated one (`build_import_map` leaves it unmapped).
            if let ImportDeclarationSpecifier::ImportNamespaceSpecifier(ns) = spec
                && let Some(source) = decl.source.value.as_str()
            {
                aliases.insert(ns.local.name.to_string(), source.to_string());
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
/// A dotted name (`NS.App`) is the class `App` of namespace `NS`. A class
/// declared in `statements` comes before one of the same name in a block.
fn find_class_body<'a, 'b>(
    statements: &'b [Statement<'a>],
    class_name: &str,
) -> Option<&'b ClassBody<'a>> {
    if class_name.contains('.') {
        let path: std::vec::Vec<&str> = class_name.split('.').collect();
        return find_qualified_class_body(statements, &path);
    }

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
        in_block: bool,
    ) -> Option<&'b ClassBody<'a>> {
        match decl {
            Declaration::ClassDeclaration(class) => {
                if !in_block && class.id.as_ref().map(|id| id.name.as_str()) == Some(class_name) {
                    Some(&*class.body)
                } else {
                    None
                }
            }
            _ if !in_block => None,
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

    for in_block in [false, true] {
        for stmt in statements {
            let found = match stmt {
                // `export declare class` — oxc splits export-with-declaration
                // into `ExportDeclaration`, while `export { a }` is
                // `ExportNamedDeclaration` (which never wraps a declaration).
                Statement::ExportDeclaration(export) => {
                    from_declaration(&export.declaration, class_name, in_block)
                }
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class)
                        if !in_block
                            && class.id.as_ref().map(|id| id.name.as_str()) == Some(class_name) =>
                    {
                        Some(&*class.body)
                    }
                    _ => None,
                },
                other => other
                    .as_declaration()
                    .and_then(|decl| from_declaration(decl, class_name, in_block)),
            };
            if found.is_some() {
                return found;
            }
        }
    }
    None
}

/// The class at `path` (`["A", "B", "C"]` for `A.B.C`): every segment but the
/// last names a namespace, in `statements` itself, and the last a class
/// declared directly in the innermost one. Namespaces of one name merge, so
/// each is tried in turn.
fn find_qualified_class_body<'a, 'b>(
    statements: &'b [Statement<'a>],
    path: &[&str],
) -> Option<&'b ClassBody<'a>> {
    fn in_namespace<'a, 'b>(
        ns: &'b oxc_ast::ast::TSNamespaceDeclaration<'a>,
        path: &[&str],
    ) -> Option<&'b ClassBody<'a>> {
        let (first, rest) = path.split_first()?;
        if ns.id.name.as_str() != *first || rest.is_empty() {
            return None;
        }
        match &ns.body {
            TSNamespaceDeclarationBody::TSModuleBlock(block) => {
                find_qualified_class_body(&block.body, rest)
            }
            TSNamespaceDeclarationBody::TSNamespaceDeclaration(inner) => in_namespace(inner, rest),
        }
    }

    statements.iter().find_map(|stmt| {
        let decl = match stmt {
            Statement::ExportDeclaration(export) => Some(&export.declaration),
            other => other.as_declaration(),
        }?;
        match decl {
            Declaration::ClassDeclaration(class)
                if path.len() == 1
                    && class.id.as_ref().map(|id| id.name.as_str()) == Some(path[0]) =>
            {
                Some(&*class.body)
            }
            Declaration::TSNamespaceDeclaration(ns) => in_namespace(ns, path),
            _ => None,
        }
    })
}

/// The member's name as written, if it is a non-computed static or string
/// literal key.
fn property_key_name(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.to_string()),
        _ => match key.as_expression() {
            Some(Expression::StringLiteral(lit)) => lit.value.as_str().map(str::to_string),
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

    /// `typeof i1["s-dir"]`, for an export named by a string: the head is
    /// the whole type query `i1`, when it is a namespace the members map.
    fn visit_ts_type_query(&mut self, node: &oxc_ast::ast::TSTypeQuery<'a>) {
        if let oxc_ast::ast::TSTypeQueryExprName::IdentifierReference(id) = &node.expr_name
            && let Some(specifier) = self.namespace_imports.get(id.name.as_str())
        {
            self.collected.push((
                id.span.start as usize - self.offset,
                id.name.to_string(),
                specifier.clone(),
                (self.resolve)(specifier),
            ));
        }
        walk::walk_ts_type_query(self, node);
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

/// Where the imports go in a file with no import: before the comments of
/// the first statement (at `statement_start`), as ngtsc's ImportManager
/// adds them in front of that statement and TS prints its leading comments
/// after them. So the statement's JSDoc stays on it: `import * as i0 …;`,
/// `/** doc */`, `export declare class X`.
///
/// Only `/// <reference …/>` and `/// <amd-…/>` directives stay above the
/// imports, as they work only at the top of the file. Every other comment,
/// a license header too, goes below them: ngc edits the declaration AST, so
/// its namespace import is always first.
fn first_import_position(source: &str, statement_start: usize, program: &Program<'_>) -> usize {
    let leading: Vec<Span> = program
        .comments
        .iter()
        .map(|comment| comment.span)
        .take_while(|span| span.end as usize <= statement_start)
        .collect();
    let text = |span: &Span| &source[span.start as usize..span.end as usize];
    // Comments up to the last `/// <…/>` directive.
    let kept = leading
        .iter()
        .rposition(|span| {
            text(span).strip_prefix("///").is_some_and(|rest| rest.trim_start().starts_with('<'))
        })
        .map_or(0, |index| index + 1);
    leading.get(kept).map_or(statement_start, |span| span.start as usize)
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
    apply_splices(source, &merge_splices(plan_splices(source, declarations)))
}

/// The output of [`inject_dts_declarations_with_map`] and
/// [`inject_dts_declarations_with_identity_map`].
pub struct DtsInjectResult {
    /// The `.d.ts` with the members and imports injected.
    pub code: String,
    /// The source map `mappings` for `code`, or `None` when there is none
    /// to give (see each function).
    pub mappings: Option<String>,
}

/// [`inject_dts_declarations`], and also shift the `mappings` of the
/// `.d.ts`'s existing declaration map so it fits the new code.
///
/// Like ngtsc, which adds the members in an `afterDeclarations` transform
/// (ngtsc/transform/src/declaration.ts) so TS writes the `.d.ts` and its map
/// together: original tokens keep their mapping, injected text has none.
/// One gap: ngtsc maps the class name inside injected members
/// (`ɵɵFactoryDeclaration<App, …>`); here it stays unmapped.
///
/// Nothing to inject gives `mappings` back as is. Bad VLQ gives `None`.
pub fn inject_dts_declarations_with_map(
    source: &str,
    declarations: &[DtsInjectDeclaration],
    mappings: Option<&str>,
) -> DtsInjectResult {
    let splices = plan_splices(source, declarations);
    if splices.is_empty() {
        return DtsInjectResult {
            code: source.to_string(),
            mappings: mappings.map(str::to_string),
        };
    }
    let splices = merge_splices(splices);
    DtsInjectResult {
        code: apply_splices(source, &splices),
        mappings: mappings.and_then(|m| shift_mappings(source, &splices, m)),
    }
}

/// [`inject_dts_declarations`], plus `mappings` from the new code back to
/// `source` (source index 0), e.g. for a bundler `transform` hook.
///
/// Every identifier run and punctuation char of `source` gets a segment;
/// injected text is unmapped. Nothing to inject gives `mappings: None`
/// (the code is unchanged).
pub fn inject_dts_declarations_with_identity_map(
    source: &str,
    declarations: &[DtsInjectDeclaration],
) -> DtsInjectResult {
    let splices = plan_splices(source, declarations);
    if splices.is_empty() {
        return DtsInjectResult { code: source.to_string(), mappings: None };
    }
    let splices = merge_splices(splices);
    DtsInjectResult {
        code: apply_splices(source, &splices),
        mappings: shift_mappings(source, &splices, &identity_mappings(source)),
    }
}

/// The insertions [`inject_dts_declarations`] makes, as `(byte position,
/// text)` in the order they were planned. Empty when nothing is injected.
fn plan_splices(source: &str, declarations: &[DtsInjectDeclaration]) -> Vec<(usize, String)> {
    if declarations.is_empty() {
        return vec![];
    }

    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, source, SourceType::d_ts()).parse();
    if !ret.diagnostics.is_empty() {
        return vec![];
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

    let mut splices: Vec<(usize, String)> = Vec::new();

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
        splices.push((insert_at, text));
    }

    if splices.is_empty() {
        return splices;
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
                format!(
                    "\n{text}{}",
                    if at_newline || position == source.len() { "" } else { "\n" }
                ),
            ));
        } else if let Some(first) = program.body.first() {
            let start = first_import_position(source, first.span().start as usize, program);
            splices.push((start, format!("{text}\n")));
        } else {
            splices.push((
                source.len(),
                format!(
                    "{}{text}\n",
                    if source.ends_with('\n') || source.is_empty() { "" } else { "\n" }
                ),
            ));
        }
    }

    splices
}

/// Sort splices by position and join the texts at one position into one.
/// There the later-planned text goes first, as the old stable
/// reverse-order apply did.
fn merge_splices(splices: Vec<(usize, String)>) -> Vec<(usize, String)> {
    let mut indexed: Vec<_> = splices.into_iter().enumerate().collect();
    indexed.sort_by_key(|(index, (pos, _))| (*pos, std::cmp::Reverse(*index)));
    let mut merged: Vec<(usize, String)> = Vec::new();
    for (_, (pos, text)) in indexed {
        match merged.last_mut() {
            Some((last, joined)) if *last == pos => joined.push_str(&text),
            _ => merged.push((pos, text)),
        }
    }
    merged
}

/// Insert merged splices (see [`merge_splices`]) into `source`.
fn apply_splices(source: &str, splices: &[(usize, String)]) -> String {
    let added: usize = splices.iter().map(|(_, text)| text.len()).sum();
    let mut output = String::with_capacity(source.len() + added);
    let mut last = 0;
    for (pos, text) in splices {
        output.push_str(&source[last..*pos]);
        output.push_str(text);
        last = *pos;
    }
    output.push_str(&source[last..]);
    output
}

/// One splice in source-map terms of the original source: lines split on
/// `\n` only (`\r` is a column char), columns in UTF-16 units.
struct SpliceShift {
    line: usize,
    col: i64,
    /// `\n`s in the inserted text.
    newlines: usize,
    /// UTF-16 length of the inserted text's last line.
    last_len: i64,
    /// The text's first line is not empty, so it sits after any code before
    /// it on that line.
    first_nonempty: bool,
}

fn utf16_len(text: &str) -> i64 {
    text.encode_utf16().count() as i64
}

/// Move the generated positions of `mappings`, written for `source`, so
/// they fit `source` with `splices` (merged, see [`merge_splices`])
/// inserted. Fields 2-5 of a segment are deltas over the whole map and the
/// segment order stays, so they are copied as they are. Injected text is
/// unmapped: where it starts after mapped code on a line, a 1-field segment
/// ends that code's mapping. `None` on bad VLQ.
///
/// One pass over `source` and the splices finds where each splice is; each
/// segment then moves in `O(log splices)`.
fn shift_mappings(source: &str, splices: &[(usize, String)], mappings: &str) -> Option<String> {
    let mut shifts: Vec<SpliceShift> = Vec::with_capacity(splices.len());
    // The scan so far: up to `scanned`, at `(line, col)`.
    let (mut scanned, mut line, mut col) = (0, 0, 0);
    for (pos, text) in splices {
        let gap = &source[scanned..*pos];
        match gap.rfind('\n') {
            Some(last) => {
                line += gap.bytes().filter(|&b| b == b'\n').count();
                col = utf16_len(&gap[last + 1..]);
            }
            None => col += utf16_len(gap),
        }
        scanned = *pos;
        let last_line_start = text.rfind('\n').map_or(0, |i| i + 1);
        shifts.push(SpliceShift {
            line,
            col,
            newlines: text.bytes().filter(|&b| b == b'\n').count(),
            last_len: utf16_len(&text[last_line_start..]),
            first_nonempty: !text.starts_with('\n'),
        });
    }
    // Over `shifts[..k]`: `newlines_before[k]` lines added, `len_before[k]`
    // the sum of `last_len`, and `last_break[k]` the last splice with a
    // line break.
    let mut newlines_before = Vec::with_capacity(shifts.len() + 1);
    let mut len_before = Vec::with_capacity(shifts.len() + 1);
    let mut last_break = Vec::with_capacity(shifts.len() + 1);
    let (mut newlines, mut len, mut last) = (0, 0, None);
    for (index, s) in shifts.iter().enumerate() {
        newlines_before.push(newlines);
        len_before.push(len);
        last_break.push(last);
        newlines += s.newlines;
        len += s.last_len;
        if s.newlines > 0 {
            last = Some(index);
        }
    }
    newlines_before.push(newlines);
    len_before.push(len);
    last_break.push(last);
    // Where an original `(line, col)` lands. A splice right at `col` moves
    // it when `at_col_moves` (the text goes in before that char). The
    // splices before it on its line apply in order: one with a line break
    // starts a new line, the others add their length.
    let shift = |line: usize, col: i64, at_col_moves: bool| -> (usize, i64) {
        let first = shifts.partition_point(|s| s.line < line);
        let end = first
            + shifts[first..].partition_point(|s| {
                s.line == line && (s.col < col || (s.col == col && at_col_moves))
            });
        let new_line = line + newlines_before[end];
        let new_col = match last_break[end] {
            Some(b) if b >= first => {
                shifts[b].last_len + (col - shifts[b].col) + len_before[end] - len_before[b + 1]
            }
            _ => col + len_before[end] - len_before[first],
        };
        (new_line, new_col)
    };

    // Per new line: (generated column, raw fields 2-5, `None` for 1-field).
    let mut lines: Vec<Vec<(i64, Option<&str>)>> = Vec::new();
    let mut line_count = 0;
    for (line, text) in mappings.split(';').enumerate() {
        line_count = line + 1;
        let mut col = 0;
        for segment in text.split(',').filter(|segment| !segment.is_empty()) {
            let bytes = segment.as_bytes();
            let mut i = 0;
            col += vlq_decode(bytes, &mut i)?;
            if col < 0 {
                return None;
            }
            let rest = &segment[i..];
            while i < bytes.len() {
                vlq_decode(bytes, &mut i)?;
            }
            let (new_line, new_col) = shift(line, col, true);
            if lines.len() <= new_line {
                lines.resize_with(new_line + 1, Vec::new);
            }
            lines[new_line].push((new_col, (!rest.is_empty()).then_some(rest)));
        }
    }
    let line_count = shift(line_count - 1, 0, false).0 + 1;
    if lines.len() < line_count {
        lines.resize_with(line_count, Vec::new);
    }

    // Where injected text starts after mapped code, a 1-field segment ends
    // that code's mapping. These come in order, so they merge into each
    // line as it is written.
    let mut breaks =
        shifts.iter().filter(|s| s.first_nonempty).map(|s| shift(s.line, s.col, false)).peekable();
    let mut merged: Vec<(i64, Option<&str>)> = Vec::new();

    let mut output = String::with_capacity(mappings.len() + lines.len());
    for (index, segments) in lines.iter().enumerate() {
        if index > 0 {
            output.push(';');
        }
        merged.clear();
        for &(col, rest) in segments.iter().chain(std::iter::once(&(i64::MAX, None))) {
            while let Some((_, at)) = breaks.next_if(|&(line, at)| line == index && at <= col) {
                if merged.last().is_some_and(|(_, rest)| rest.is_some()) {
                    merged.push((at, None));
                }
            }
            if col != i64::MAX {
                merged.push((col, rest));
            }
        }
        let mut prev = 0;
        for (segment_index, (col, rest)) in merged.iter().enumerate() {
            if segment_index > 0 {
                output.push(',');
            }
            vlq_encode(&mut output, col - prev);
            prev = *col;
            if let Some(rest) = rest {
                output.push_str(rest);
            }
        }
    }
    Some(output)
}

/// A map of `source` onto itself (source index 0): a segment at the start
/// of every identifier run and at every other non-space char.
fn identity_mappings(source: &str) -> String {
    let mut output = String::new();
    let (mut prev_line, mut prev_col) = (0, 0);
    for (line, text) in source.split('\n').enumerate() {
        if line > 0 {
            output.push(';');
        }
        let line = line as i64;
        let (mut col, mut generated_prev, mut in_word, mut first) = (0, 0, false, true);
        for ch in text.chars() {
            let word = ch.is_alphanumeric() || ch == '_' || ch == '$';
            if !(ch.is_whitespace() || (word && in_word)) {
                if !first {
                    output.push(',');
                }
                first = false;
                vlq_encode(&mut output, col - generated_prev);
                vlq_encode(&mut output, 0);
                vlq_encode(&mut output, line - prev_line);
                vlq_encode(&mut output, col - prev_col);
                (generated_prev, prev_line, prev_col) = (col, line, col);
            }
            in_word = word;
            col += ch.len_utf16() as i64;
        }
    }
    output
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Read one base64 VLQ value at `bytes[*i..]`.
fn vlq_decode(bytes: &[u8], i: &mut usize) -> Option<i64> {
    let mut value: i64 = 0;
    let mut shift = 0;
    loop {
        let byte = *bytes.get(*i)?;
        *i += 1;
        let digit = BASE64.iter().position(|&b| b == byte)? as i64;
        if shift > 55 {
            return None;
        }
        value |= (digit & 31) << shift;
        if digit & 32 == 0 {
            break;
        }
        shift += 5;
    }
    Some(if value & 1 == 1 { -(value >> 1) } else { value >> 1 })
}

fn vlq_encode(output: &mut String, value: i64) {
    let mut rest = if value < 0 { (value.unsigned_abs() << 1) | 1 } else { (value as u64) << 1 };
    loop {
        let mut digit = (rest & 31) as usize;
        rest >>= 5;
        if rest > 0 {
            digit |= 32;
        }
        output.push(BASE64[digit] as char);
        if rest == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The apply used before splices were merged: stable sort by reverse
    /// position, then insert one by one.
    fn old_apply(source: &str, splices: &[(usize, String)]) -> String {
        let mut splices = splices.to_vec();
        splices.sort_by_key(|(pos, _)| std::cmp::Reverse(*pos));
        let mut output = source.to_string();
        for (pos, text) in splices {
            output.insert_str(pos, &text);
        }
        output
    }

    #[test]
    fn merge_keeps_the_old_order_at_one_position() {
        let source = "abc";
        let splices: Vec<(usize, String)> = vec![
            (1, "1".into()),
            (3, "2".into()),
            (1, "3".into()),
            (0, "4".into()),
            (1, "5".into()),
        ];
        assert_eq!(
            apply_splices(source, &merge_splices(splices.clone())),
            old_apply(source, &splices)
        );

        // The same class twice gives two splices at one position.
        let declaration = || DtsInjectDeclaration {
            class_name: "X".into(),
            members: "static ɵfac: i0.ɵɵFactoryDeclaration<X, never>;".into(),
            namespace_imports: [("i0".to_string(), CORE.to_string())].into_iter().collect(),
            source_file: None,
        };
        let source = "export declare class X {}\n";
        let splices = plan_splices(source, &[declaration(), declaration()]);
        assert_eq!(splices.len(), 3);
        assert_eq!(
            apply_splices(source, &merge_splices(splices.clone())),
            old_apply(source, &splices)
        );
    }

    #[test]
    fn imports_the_namespace_of_an_indexed_type_query() {
        // `typeof i1["s-dir"]` (an export named by a string) is `i1` alone
        // in a type query; it is canonicalized and imported like `i1.X`.
        let declaration = DtsInjectDeclaration {
            class_name: "M".into(),
            members:
                "static ɵmod: i0.ɵɵNgModuleDeclaration<M, never, [typeof i1[\"s-dir\"]], never>;"
                    .into(),
            namespace_imports: [
                ("i0".to_string(), CORE.to_string()),
                ("i1".to_string(), "./dep".to_string()),
            ]
            .into_iter()
            .collect(),
            source_file: None,
        };
        let source = "import { i1 } from \"./other\";\nexport declare class M {\n}\n";
        assert_eq!(
            inject_dts_declarations(source, &[declaration]),
            "import { i1 } from \"./other\";\n\
             import * as i0 from \"@angular/core\";\n\
             import * as i1_1 from \"./dep\";\n\
             export declare class M {\n    \
             static ɵmod: i0.ɵɵNgModuleDeclaration<M, never, [typeof i1_1[\"s-dir\"]], never>;\n\
             }\n"
        );
    }

    #[test]
    fn shifts_by_the_utf16_length_of_injected_text() {
        // `ɵ` is 1 UTF-16 unit (2 bytes), `𝒳` is 2 (4 bytes); `a` at 0,
        // `;` at 1, `b` at 2.
        let source = "a;b";
        let splices = vec![(1, "ɵ𝒳".to_string())];
        assert_eq!(
            shift_mappings(source, &splices, "AAAA,CAAC,CAAC").as_deref(),
            Some("AAAA,C,GAAC,CAAC")
        );
        // Text that ends a line: `;` goes to col 3 of the next line.
        let splices = vec![(1, "\nɵ𝒳".to_string())];
        assert_eq!(
            shift_mappings(source, &splices, "AAAA,CAAC,CAAC").as_deref(),
            Some("AAAA;GAAC,CAAC")
        );
    }

    #[test]
    fn shifts_through_several_splices_on_one_line() {
        // `x` before `b`, a line break before `c`: `a` stays, `b` goes from
        // 2 to 3 (a 1-field segment at 2), `c` to the next line at col 0.
        let source = "a b c";
        let splices = vec![(2, "x".to_string()), (4, "\n".to_string())];
        assert_eq!(
            shift_mappings(source, &splices, "AAAA,EAAE,EAAE").as_deref(),
            Some("AAAA,E,CAAE;AAAE")
        );
        // A line break, then text after it: `b` goes to (1, 2), `z` ends
        // its mapping at (1, 4), `c` goes to (1, 5) and `d` to (1, 7).
        let source = "a b c d";
        let splices = vec![(2, "\nxy".to_string()), (4, "z".to_string())];
        assert_eq!(
            shift_mappings(source, &splices, "AAAA,EAAE,EAAE,EAAE").as_deref(),
            Some("AAAA;EAAE,E,CAAE,EAAE")
        );
    }
}
