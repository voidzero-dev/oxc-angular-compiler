//! Prints a TypeScript type the way ngtsc writes it into a `.d.ts`: through
//! its `TypeEmitter` (every reference to an `@angular/core` name becomes
//! `i0.Name`, literals are re-created) and TypeScript's printer (normalised
//! spacing, double-quoted ASCII-only strings, only some comments kept).
//!
//! Used for `static ngAcceptInputType_x: T;`, where `T` is the type of the
//! input transform's first parameter.
//!
//! Every part of the type is printed from the AST, never copied from the
//! source, so no position can keep a name ngtsc would have rewritten. A form
//! this printer doesn't cover makes the whole type `unknown`.
//!
//! Comments follow TypeScript's declaration emit (`onlyPrintJsDocStyle`):
//! node-level leading/trailing comment scans keep `/**`-style (and `/*!`)
//! comments only, while the intervening scan inside a list keeps every
//! comment — which is also why a comment can print twice, once per enclosing
//! list's intervening scan. Which trivia a scan sees comes from
//! `iterateCommentRanges`: a trailing scan collects comments on `pos`'s
//! line, a leading one everything after the first `\r`/`\n` — except that
//! U+2028/U+2029 are whitespace for the scan while the line map counts them
//! as line breaks.
//!
//! ngtsc's `markForEmitAsSingleLine` marks the whole translated type
//! subtree. This printer always uses the single-line formats — but ngtsc's
//! `ts.transform` calls `disposeEmitNodes`, which wipes the marks on nodes
//! visited before the last `emitType` run, so in real ngtsc output a field
//! whose type is a bare tuple, type literal or mapped type can print
//! multi-line (while `i0.Foo<…>` references stay single-line via the
//! checker's reuse path). That divergence predates the comment support and
//! needs the field-order mechanism to fix; the single-line path is what the
//! fixtures exercise. Either way the comment paths need the writer's line
//! tracking: a `//` comment or a `/**` with `hasTrailingNewLine` ends the
//! line, and the next write indents it.

use oxc_ast::ast::{
    BigIntLiteral, BindingPattern, BindingProperty, BindingRestElement, Comment as OxcComment,
    CommentKind, Expression, FormalParameter, FormalParameterRest, FormalParameters, PropertyKey,
    StringLiteral, TSIndexSignatureName, TSLiteral, TSMappedTypeModifierOperator,
    TSMethodSignatureKind, TSNamedTupleMember, TSSignature, TSThisParameter, TSTupleElement,
    TSType, TSTypeAnnotation, TSTypeName, TSTypeOperatorOperator, TSTypeParameter,
    TSTypeParameterDeclaration, TSTypeParameterInstantiation, TSTypePredicateName,
    TSTypeQueryExprName, UnaryOperator,
};
use oxc_span::{GetSpan, Span};

use super::evaluator::{AliasTarget, FileScope};
use crate::output::emitter::format_number_like_js;
use crate::r3::CORE;

/// TypeScript's `ListFormat` bits (`emitter.ts`) for the lists this printer
/// emits. ngtsc marks the type `EmitFlags.SingleLine`, so only the
/// single-line formats are reachable: `MultiLine`/`Indented` are defined for
/// completeness but never engage — which is also why `increaseIndent` never
/// runs and the writer indent stays at one level (four spaces).
mod fmt {
    pub const BAR: u32 = 4;
    pub const AMPERSAND: u32 = 8;
    pub const COMMA: u32 = 16;
    pub const ALLOW_TRAILING_COMMA: u32 = 64;
    pub const INDENTED: u32 = 128;
    pub const SPACE_BETWEEN_BRACES: u32 = 256;
    pub const SPACE_BETWEEN_SIBLINGS: u32 = 512;
    pub const PARENTHESIS: u32 = 1024;
    pub const ANGLE: u32 = 2048;
    pub const SQUARE: u32 = 8192;
    pub const OPTIONAL_UNDEFINED: u32 = 16384;
    pub const OPTIONAL_EMPTY: u32 = 32768;
    pub const NO_INTERVENING: u32 = 262144;
    pub const NO_SPACE_IF_EMPTY: u32 = 524288;
    pub const SPACE_AFTER_LIST: u32 = 2097152;

    pub const DELIMITERS: u32 = BAR | AMPERSAND | COMMA | 32; // DelimitersMask
    pub const BRACKETS: u32 = PARENTHESIS | ANGLE | 4096 | SQUARE; // BracketsMask

    /// `UnionTypeConstituents` / `IntersectionTypeConstituents`.
    pub const UNION: u32 = BAR | SPACE_BETWEEN_SIBLINGS;
    pub const INTERSECTION: u32 = AMPERSAND | SPACE_BETWEEN_SIBLINGS;
    /// `TypeParameters` / `TypeArguments`.
    pub const TYPE_ARGS: u32 =
        OPTIONAL_UNDEFINED | OPTIONAL_EMPTY | ANGLE | COMMA | SPACE_BETWEEN_SIBLINGS;
    /// `Parameters`.
    pub const PARAMETERS: u32 = PARENTHESIS | COMMA | SPACE_BETWEEN_SIBLINGS;
    /// `IndexSignatureParameters`.
    pub const INDEX_PARAMETERS: u32 = SQUARE | INDENTED | COMMA | SPACE_BETWEEN_SIBLINGS;
    /// `SingleLineTypeLiteralMembers` (`NoSpaceIfEmpty` is added at the site).
    pub const TYPE_LITERAL_MEMBERS: u32 = SPACE_BETWEEN_BRACES | SPACE_BETWEEN_SIBLINGS;
    /// `SingleLineTupleTypeElements` (`NoSpaceIfEmpty` is added at the site).
    pub const TUPLE_ELEMENTS: u32 = COMMA | SPACE_BETWEEN_SIBLINGS;
    /// `ObjectBindingPatternElements`.
    pub const OBJECT_BINDING: u32 = SPACE_BETWEEN_BRACES
        | COMMA
        | ALLOW_TRAILING_COMMA
        | SPACE_BETWEEN_SIBLINGS
        | NO_SPACE_IF_EMPTY;
    /// `ArrayBindingPatternElements`.
    pub const ARRAY_BINDING: u32 =
        COMMA | ALLOW_TRAILING_COMMA | SPACE_BETWEEN_SIBLINGS | NO_SPACE_IF_EMPTY;
    /// `Modifiers`: a space-separated, comment-free list.
    pub const MODIFIERS: u32 = SPACE_BETWEEN_SIBLINGS | NO_INTERVENING | SPACE_AFTER_LIST;
}

/// The writer indent (`getIndentString`): `static` members sit at one level.
const INDENT: &str = "    ";

pub(crate) struct TypePrinter<'s, 'a> {
    pub scope: &'s FileScope<'a>,
    pub source: &'a str,
    /// The alias `@angular/core` names are printed through — the namespace
    /// the file's `ImportManager` picked (`i0`, or `i0_1`, … when a user
    /// binding collided), like ngtsc's `TypeEmitter` qualifier.
    pub core_ns: &'a str,
    /// Set when the type references a module other than `@angular/core`.
    /// ngtsc would add an `import * as iN` for it; oxc emits `unknown` instead,
    /// since aliases numbered per source file can't be merged into bundled
    /// declaration files safely.
    pub other_module: bool,
    /// TypeScript's printer dedupes comments through `containerPos` /
    /// `containerEnd`: a node's leading comments are skipped when its `pos`
    /// (trivia start) is the enclosing emitted node's, and its trailing
    /// comments when its `end` is the enclosing node's. These hold the
    /// enclosing node's range while [`Self::emit_node`] prints one.
    pub container_pos: u32,
    pub container_end: u32,
    /// The writer's indent level (`createTextWriter`'s `indent`): `static`
    /// members sit at one level, and a `ListFormat.Indented` list adds one
    /// more while its items print.
    pub indent: u32,
    /// The source's comment table, shared across the printers of one
    /// class: the backward trivia scan queries it instead of reparsing the
    /// whole prefix per node.
    pub(crate) lexed: std::rc::Rc<Lexed>,
}

/// A list element for [`TypePrinter::emit_list`]: a node with a span plus the
/// body that prints it, like `emitListItem` calling `emit` on a `TSSignature`,
/// `Parameter`, `TypeNode`, …
enum El<'n, 'a> {
    Ty(&'n TSType<'a>),
    /// A `TSTupleElement` (`TSOptionalType`/`TSRestType`/`TSType`).
    TupleEl(&'n TSTupleElement<'a>),
    /// A `TSSignature` member of a type literal.
    Member(&'n TSSignature<'a>),
    /// A `TSTypeParameter`.
    TypeParam(&'n TSTypeParameter<'a>),
    /// A `FormalParameter`.
    Param(&'n FormalParameter<'a>),
    /// The `this` parameter.
    This(&'n TSThisParameter<'a>),
    /// A `FormalParameterRest`.
    Rest(&'n FormalParameterRest<'a>),
    /// An index signature's `k: T` (`TSIndexSignatureName`, a `Parameter`).
    IndexParam(&'n TSIndexSignatureName<'a>),
    /// A `BindingPattern` (a `BindingElement`'s name).
    Binding(&'n BindingPattern<'a>),
    /// A `BindingProperty` (object-pattern `key: name` or shorthand `name`).
    BindingProp(&'n BindingProperty<'a>),
    /// A `BindingRestElement` (`...arg`).
    BindingRest(&'n BindingRestElement<'a>),
    /// A span-printed token: a modifier keyword, a `?`, or an omitted array
    /// element (zero length at the comma).
    Token {
        span: Span,
        text: &'static str,
    },
}

impl El<'_, '_> {
    fn span(&self) -> Span {
        match self {
            El::Ty(t) => t.span(),
            El::TupleEl(e) => e.span(),
            El::Member(m) => m.span(),
            El::TypeParam(t) => t.span,
            El::Param(p) => p.span,
            El::This(t) => t.span,
            El::Rest(r) => r.span,
            El::IndexParam(p) => p.span,
            El::Binding(b) => b.span(),
            El::BindingProp(p) => p.span,
            El::BindingRest(r) => r.span,
            El::Token { span, .. } => *span,
        }
    }
}

impl<'a> TypePrinter<'_, 'a> {
    /// `unknown` when the type has a form ngtsc can't emit either (an
    /// `import('...')` type) or that isn't valid in a type annotation.
    pub(crate) fn print(&mut self, ty: &TSType<'a>) -> String {
        let mut out = String::new();
        match self.ty_node(&mut out, ty) {
            Some(()) => out,
            None => "unknown".to_string(),
        }
    }

    /// A node's `pos` (start of its leading trivia): the end of the previous
    /// token.
    fn pos_of(&self, start: u32) -> u32 {
        token_end_before(&self.lexed, self.source, start as usize).unwrap_or(start as usize) as u32
    }

    /// Emit a node like TypeScript's `emit(node)`: its leading comments, the
    /// node body, then its trailing comments — `emitCommentsBeforeNode` /
    /// `emitCommentsAfterNode` around every emitted node. `pos` is the trivia
    /// start and `end` the token end; the scans dedupe against the enclosing
    /// node through `container_pos` / `container_end`. With `pos == end` no
    /// comments run (`emitComments*` skip that range) and nothing wraps
    /// `body`.
    fn emit_node(
        &mut self,
        out: &mut String,
        pos: u32,
        end: u32,
        body: impl FnOnce(&mut Self, &mut String) -> Option<()>,
    ) -> Option<()> {
        if pos == end {
            return body(self, out);
        }
        self.emit_leading_of_pos(out, pos as usize);
        let (saved_pos, saved_end) = (self.container_pos, self.container_end);
        self.container_pos = pos;
        self.container_end = end;
        let ok = body(self, out);
        // Trailing comments compare against the *enclosing* node's range —
        // the saved values are restored first, like `emitTrailingCommentsOfNode`.
        self.container_pos = saved_pos;
        self.container_end = saved_end;
        self.emit_trailing(out, end as usize);
        ok
    }

    /// [`Self::emit_node`] for a `TSType`.
    fn ty_node(&mut self, out: &mut String, ty: &TSType<'a>) -> Option<()> {
        let pos = self.pos_of(ty.span().start);
        self.emit_node(out, pos, ty.span().end, |s, o| s.ty_body(o, ty))
    }

    /// `write`/`writeComment`/`writeSpace`/`writePunctuation`: the text, with
    /// the writer indent prepended at a line start (after a `writeLine`,
    /// which comments emit).
    fn write(&self, out: &mut String, s: &str) {
        if out.ends_with('\n') {
            for _ in 0..self.indent {
                out.push_str(INDENT);
            }
        }
        out.push_str(s);
    }

    /// `emitNodeList` + `emitNodeListItems`.
    ///
    /// `items` are the list's children; `list_pos`/`list_end` are the
    /// `NodeArray`'s trivia range (inside the brackets — used only for the
    /// empty-list comment scans, and `list_end` again for the closing scan
    /// after a trailing comma); `parent_end` is the enclosing node's `end`
    /// — the gate that keeps the comments after the last element out when the
    /// parent ends there too. `trailing_comma` is the `NodeArray`'s
    /// `hasTrailingComma` — written only when the format allows it.
    /// `open`/`close` are the bracket texts for `BracketsMask` formats.
    fn emit_list(
        &mut self,
        out: &mut String,
        items: &[El<'_, 'a>],
        format: u32,
        parent_end: u32,
        list_pos: usize,
        list_end: usize,
        trailing_comma: bool,
        open: Option<&'static str>,
        close: Option<&'static str>,
    ) -> Option<()> {
        if items.is_empty() && format & fmt::OPTIONAL_EMPTY != 0 {
            return Some(());
        }
        if format & fmt::BRACKETS != 0 {
            self.write(out, open.expect("bracketed format"));
            if items.is_empty() {
                // `emitTrailingCommentsOfPosition(children.pos, prefixSpace)`:
                // JSDoc comments inside the empty brackets.
                self.emit_trailing_filtered(out, list_pos);
            }
        }
        if items.is_empty() {
            // The MultiLine branch never runs (single-line formats only).
            if format & fmt::SPACE_BETWEEN_BRACES != 0 && format & fmt::NO_SPACE_IF_EMPTY == 0 {
                self.write(out, " ");
            }
        } else {
            self.emit_list_items(out, items, format, parent_end, list_end, trailing_comma)?;
        }
        if format & fmt::BRACKETS != 0 {
            if items.is_empty() {
                // `emitLeadingCommentsOfPosition(children.end)`.
                self.emit_leading_of_pos(out, list_end);
            }
            self.write(out, close.expect("bracketed format"));
        }
        Some(())
    }

    /// `emitNodeListItems`, always on the single-line path: every list this
    /// printer uses is `SingleLine`-flagged, so the leading, separating and
    /// closing line-terminator counts are all `0`. `Indented` still applies —
    /// it's unconditional in TypeScript (index-signature parameters carry it).
    fn emit_list_items(
        &mut self,
        out: &mut String,
        items: &[El<'_, 'a>],
        format: u32,
        parent_end: u32,
        list_end: usize,
        trailing_comma: bool,
    ) -> Option<()> {
        let may_intervene = format & fmt::NO_INTERVENING == 0;
        let mut should_intervene = may_intervene;
        if format & fmt::SPACE_BETWEEN_BRACES != 0 {
            self.write(out, " ");
        }
        // `Indented` applies unconditionally (index-signature parameters
        // carry it), unlike the line-terminator counts that only exist for
        // multi-line formats.
        if format & fmt::INDENTED != 0 {
            self.indent += 1;
        }
        let mut prev_end: Option<u32> = None;
        for item in items {
            let span = item.span();
            let pos = self.pos_of(span.start);
            if prev_end.is_some() {
                // The JSDoc comments between the previous element and the
                // delimiter, then the delimiter, then the separator space
                // (the separating-line-terminator branch never fires).
                if format & fmt::DELIMITERS != 0 && prev_end != Some(parent_end) {
                    self.emit_leading_of_pos(out, prev_end.unwrap() as usize);
                }
                self.write_delimiter(out, format);
                if format & fmt::SPACE_BETWEEN_SIBLINGS != 0 {
                    self.write(out, " ");
                }
            }
            if should_intervene {
                // `emitTrailingCommentsOfPosition(child.pos)`: the
                // unfiltered same-line scan — this is what keeps `/* */`
                // comments the node-level paths drop.
                self.emit_intervening(out, pos as usize);
            } else {
                should_intervene = may_intervene;
            }
            self.emit_item(out, item, pos)?;
            prev_end = Some(span.end);
        }
        // `emitTrailingComma`: the comma itself goes through
        // `emitTokenWithComment` on the last element — the comments before it
        // run only when that element has a real range (a zero-width elision
        // has `pos == end`, which suppresses the leading scan upstream), and
        // the comments after it are the comma's same-line scan. With a
        // trailing comma the closing scan runs at the list's own `end` (the
        // `NodeArray` ends inside the brackets), not the last element's.
        let emit_trailing_comma =
            trailing_comma && format & fmt::ALLOW_TRAILING_COMMA != 0 && format & fmt::COMMA != 0;
        if let Some(prev) = items.last()
            && emit_trailing_comma
            && let Some(prev_end) = prev_end
        {
            let comma = skip_trivia(self.source, prev_end as usize) + 1;
            if prev.span().start != prev.span().end && prev_end != parent_end {
                self.emit_leading_of_pos(out, prev_end as usize);
            }
            self.write(out, ",");
            if prev_end != parent_end {
                self.emit_trailing_filtered(out, comma);
            }
        }
        if let Some(prev_end) = prev_end
            && prev_end != parent_end
            && format & fmt::DELIMITERS != 0
        {
            // The JSDoc comments after the last element, before the closing
            // token.
            self.emit_leading_of_pos(
                out,
                if emit_trailing_comma { list_end } else { prev_end as usize },
            );
        }
        if format & fmt::INDENTED != 0 {
            self.indent -= 1;
        }
        if format & (fmt::SPACE_AFTER_LIST | fmt::SPACE_BETWEEN_BRACES) != 0 {
            self.write(out, " ");
        }
        Some(())
    }

    /// `writeDelimiter`: the separator token and its leading space.
    fn write_delimiter(&self, out: &mut String, format: u32) {
        match format & fmt::DELIMITERS {
            d if d == fmt::COMMA => self.write(out, ","),
            d if d == fmt::BAR => {
                self.write(out, " ");
                self.write(out, "|");
            }
            d if d == fmt::AMPERSAND => {
                self.write(out, " ");
                self.write(out, "&");
            }
            _ => {}
        }
    }

    /// `emitListItem` for one [`El`]: `emit(node)`, with the comment range of
    /// the node itself.
    fn emit_item(&mut self, out: &mut String, item: &El<'_, 'a>, pos: u32) -> Option<()> {
        let end = item.span().end;
        match item {
            El::Ty(ty) => self.ty_node(out, ty),
            El::TupleEl(e) => self.emit_node(out, pos, end, |s, o| s.tuple_element_body(o, e)),
            El::Member(m) => self.emit_node(out, pos, end, |s, o| s.member_body(o, m)),
            El::TypeParam(tp) => self.emit_node(out, pos, end, |s, o| s.type_param_body(o, tp)),
            El::Param(p) => self.emit_node(out, pos, end, |s, o| s.param_body(o, p)),
            El::This(t) => self.emit_node(out, pos, end, |s, o| s.this_body(o, t)),
            El::Rest(r) => self.emit_node(out, pos, end, |s, o| s.rest_body(o, r)),
            El::IndexParam(p) => self.emit_node(out, pos, end, |s, o| s.index_param_body(o, p)),
            El::Binding(b) => self.emit_node(out, pos, end, |s, o| s.binding_body(o, b)),
            El::BindingProp(p) => self.emit_node(out, pos, end, |s, o| s.binding_prop_body(o, p)),
            El::BindingRest(r) => self.emit_node(out, pos, end, |s, o| s.binding_rest_body(o, r)),
            El::Token { text, .. } => self.emit_node(out, pos, end, |s, o| {
                s.write(o, text);
                Some(())
            }),
        }
    }

    /// A type's body (`emit<Kind>` minus the comment pipeline, which
    /// [`Self::emit_node`] supplies).
    fn ty_body(&mut self, out: &mut String, ty: &TSType<'a>) -> Option<()> {
        match ty {
            TSType::TSAnyKeyword(_) => self.write(out, "any"),
            TSType::TSBigIntKeyword(_) => self.write(out, "bigint"),
            TSType::TSBooleanKeyword(_) => self.write(out, "boolean"),
            TSType::TSIntrinsicKeyword(_) => self.write(out, "intrinsic"),
            TSType::TSNeverKeyword(_) => self.write(out, "never"),
            TSType::TSNullKeyword(_) => self.write(out, "null"),
            TSType::TSNumberKeyword(_) => self.write(out, "number"),
            TSType::TSObjectKeyword(_) => self.write(out, "object"),
            TSType::TSStringKeyword(_) => self.write(out, "string"),
            TSType::TSSymbolKeyword(_) => self.write(out, "symbol"),
            TSType::TSUndefinedKeyword(_) => self.write(out, "undefined"),
            TSType::TSUnknownKeyword(_) => self.write(out, "unknown"),
            TSType::TSVoidKeyword(_) => self.write(out, "void"),
            TSType::TSThisType(_) => self.write(out, "this"),
            TSType::TSTypeReference(r) => {
                // ngtsc synthesizes the name (`createTypeReferenceNode`):
                // a synthesized node's `pos`/`end` are -1, so it keeps no
                // comments of its own — `Signal /** j */ <number>` prints
                // `i0.Signal<number>`. (Upstream may reuse a declaration's
                // identifier and leak *its* trivia; a reference only knows
                // the use site, so comments it would carry are dropped.)
                let text = self.type_name(&r.type_name)?;
                self.emit_node(out, 0, 0, |s, o| {
                    s.write(o, &text);
                    Some(())
                })?;
                if let Some(args) = &r.type_arguments {
                    self.type_args(out, args)?;
                }
            }
            TSType::TSUnionType(u) => {
                let items: Vec<El> = u.types.iter().map(El::Ty).collect();
                self.emit_list(out, &items, fmt::UNION, u.span.end, 0, 0, false, None, None)?;
            }
            TSType::TSIntersectionType(i) => {
                let items: Vec<El> = i.types.iter().map(El::Ty).collect();
                self.emit_list(
                    out,
                    &items,
                    fmt::INTERSECTION,
                    i.span.end,
                    0,
                    0,
                    false,
                    None,
                    None,
                )?;
            }
            TSType::TSParenthesizedType(p) => {
                self.write(out, "(");
                self.ty_node(out, &p.type_annotation)?;
                self.write(out, ")");
            }
            TSType::TSArrayType(a) => {
                self.ty_node(out, &a.element_type)?;
                self.write(out, "[]");
            }
            TSType::TSIndexedAccessType(i) => {
                self.ty_node(out, &i.object_type)?;
                self.write(out, "[");
                self.ty_node(out, &i.index_type)?;
                self.write(out, "]");
            }
            TSType::TSTypeOperatorType(o) => {
                let op = match o.operator {
                    TSTypeOperatorOperator::Keyof => "keyof",
                    TSTypeOperatorOperator::Unique => "unique",
                    TSTypeOperatorOperator::Readonly => "readonly",
                };
                self.write(out, op);
                self.write(out, " ");
                self.ty_node(out, &o.type_annotation)?;
            }
            TSType::TSTupleType(t) => {
                // `emitTokenWithComment` for both brackets: `[` carries its
                // trailing same-line comments, `]` the JSDoc ones before it.
                self.token_with_comment(
                    out,
                    "[",
                    self.pos_of(t.span.start),
                    t.span.start + 1,
                    t.span.end,
                )?;
                let items: Vec<El> = t.element_types.iter().map(El::TupleEl).collect();
                let list_pos = t.span.start as usize + 1;
                // A `NodeArray`'s `end` is past a trailing comma.
                let list_end = t
                    .element_types
                    .last()
                    .map(|e| {
                        let after = skip_trivia(self.source, e.span().end as usize);
                        if self.source[after..].starts_with(',') { after + 1 } else { after }
                    })
                    .unwrap_or(list_pos);
                self.emit_list(
                    out,
                    &items,
                    fmt::TUPLE_ELEMENTS | fmt::NO_SPACE_IF_EMPTY,
                    t.span.end,
                    list_pos,
                    list_end,
                    false,
                    None,
                    None,
                )?;
                // `emitTokenWithComment(CloseBracket, elements.end, node)`:
                // the leading scan runs at the element list's end — past a
                // trailing comma, so `[a: string,\n /** j */]` keeps the
                // comment; `]`'s own end is the tuple's, so no trailing scan.
                self.token_with_comment(out, "]", list_end as u32, t.span.end - 1, t.span.end)?;
            }
            TSType::TSNamedTupleMember(m) => self.named_tuple_member_body(out, m, None)?,
            TSType::TSLiteralType(l) => self.literal_body(out, &l.literal)?,
            TSType::TSTemplateLiteralType(t) => {
                // `emitTemplateType` + `emitLiteral`: each quasi goes out as
                // ONE `writeStringLiteral` — `` `text${` ``, `}text${`,
                // `}text`` `` — so a line break inside the text never starts
                // a write (no indent): `` `a\n${string}` `` prints `${` at
                // column 0. `quasi.span` covers only the cooked text, so a
                // chunk is the source slice from the `` ` ``/`}` before it
                // to past the `${`/`` ` `` after it. Emitting each chunk as
                // a node keeps the comment scans — the `/** j */` in
                // `` `${ /** j */ string}` `` is the head's trailing
                // comment.
                for (i, quasi) in t.quasis.iter().enumerate() {
                    let start = quasi.span.start as usize - 1;
                    let end = quasi.span.end as usize + if i + 1 == t.quasis.len() { 1 } else { 2 };
                    let chunk = self.source[start..end].to_string();
                    self.emit_node(out, self.pos_of(start as u32), end as u32, |s, o| {
                        s.write(o, &chunk);
                        Some(())
                    })?;
                    if let Some(ty) = t.types.get(i) {
                        self.ty_node(out, ty)?;
                    }
                }
            }
            TSType::TSTypeQuery(q) => {
                // `typeof x` names a value; ngtsc emits the entity name part
                // by part (`emitEntityName`), so comments inside the name are
                // kept (`typeof val.a.\n/** j */\nb`).
                self.write(out, "typeof ");
                self.emit_query_name(out, &q.expr_name)?;
                if let Some(args) = &q.type_arguments {
                    self.type_args(out, args)?;
                }
            }
            TSType::TSTypeLiteral(l) => {
                self.write(out, "{");
                let items: Vec<El> = l.members.iter().map(El::Member).collect();
                self.emit_list(
                    out,
                    &items,
                    fmt::TYPE_LITERAL_MEMBERS | fmt::NO_SPACE_IF_EMPTY,
                    l.span.end,
                    l.span.start as usize + 1,
                    l.span.end as usize - 1,
                    false,
                    None,
                    None,
                )?;
                self.write(out, "}");
            }
            TSType::TSMappedType(m) => {
                // `SingleLine` is always set: `{ ...; }` on one line.
                self.write(out, "{ ");
                if let Some(readonly) = &m.readonly {
                    // `readonlyToken` is `ReadonlyKeyword | PlusToken
                    // ReadonlyKeyword | MinusToken ReadonlyKeyword` — the sign
                    // and `readonly` are separate tokens (`{ - /** j */
                    // readonly … }` prints `- /** j */readonly`).
                    let mut start = skip_trivia(self.source, m.span.start as usize + 1);
                    if readonly != &TSMappedTypeModifierOperator::True {
                        let sign =
                            if readonly == &TSMappedTypeModifierOperator::Plus { "+" } else { "-" };
                        self.emit_node(
                            out,
                            self.pos_of(start as u32),
                            start as u32 + 1,
                            |s, o| {
                                s.write(o, sign);
                                Some(())
                            },
                        )?;
                        start = skip_trivia(self.source, start + 1);
                    }
                    // `emit(node.readonlyToken)` + `writeSpace`.
                    self.emit_node(
                        out,
                        self.pos_of(start as u32),
                        (start + "readonly".len()) as u32,
                        |s, o| {
                            s.write(o, "readonly");
                            Some(())
                        },
                    )?;
                    self.write(out, " ");
                }
                self.write(out, "[");
                // `emitMappedTypeParameter`: `K in T` (the mapped modifiers
                // live outside this node in TypeScript's AST too).
                self.emit_node(
                    out,
                    self.pos_of(m.key.span.start),
                    m.constraint.span().end,
                    |s, o| {
                        s.emit_node(o, s.pos_of(m.key.span.start), m.key.span.end, |s2, o2| {
                            s2.write(o2, &m.key.name);
                            Some(())
                        })?;
                        s.write(o, " in ");
                        s.ty_node(o, &m.constraint)
                    },
                )?;
                let mut after = m.constraint.span().end;
                if let Some(name_type) = &m.name_type {
                    self.write(out, " as ");
                    self.ty_node(out, name_type)?;
                    after = name_type.span().end;
                }
                self.write(out, "]");
                if let Some(optional) = &m.optional {
                    // `questionToken` is `QuestionToken | PlusToken
                    // QuestionToken | MinusToken QuestionToken` — separate
                    // tokens, like `readonlyToken` above.
                    let mut start =
                        skip_trivia(self.source, skip_trivia(self.source, after as usize) + 1);
                    if optional != &TSMappedTypeModifierOperator::True {
                        let sign =
                            if optional == &TSMappedTypeModifierOperator::Plus { "+" } else { "-" };
                        self.emit_node(
                            out,
                            self.pos_of(start as u32),
                            start as u32 + 1,
                            |s, o| {
                                s.write(o, sign);
                                Some(())
                            },
                        )?;
                        start = skip_trivia(self.source, start + 1);
                    }
                    // `emit(node.questionToken)`: the `?` after `]`.
                    self.emit_node(out, self.pos_of(start as u32), start as u32 + 1, |s, o| {
                        s.write(o, "?");
                        Some(())
                    })?;
                }
                self.write(out, ": ");
                if let Some(value) = &m.type_annotation {
                    self.ty_node(out, value)?;
                }
                self.write(out, "; }");
            }
            TSType::TSFunctionType(f) => {
                // `emitFunctionTypeHead` + `emitFunctionTypeBody`.
                self.type_params(out, f.type_parameters.as_deref())?;
                self.params(out, f.this_param.as_deref(), &f.params, f.span.end)?;
                self.write(out, " => ");
                self.ty_node(out, &f.return_type.type_annotation)?;
            }
            TSType::TSConstructorType(c) => {
                // `emitModifierList` (`abstract`), then `new `.
                if c.r#abstract {
                    let start = skip_trivia(self.source, c.span.start as usize);
                    let mods = [El::Token {
                        span: Span::new(start as u32, start as u32 + 8),
                        text: "abstract",
                    }];
                    self.emit_list(
                        out,
                        &mods,
                        fmt::MODIFIERS,
                        c.span.end,
                        start + 8,
                        start + 8,
                        false,
                        None,
                        None,
                    )?;
                }
                self.write(out, "new ");
                self.type_params(out, c.type_parameters.as_deref())?;
                self.params(out, None, &c.params, c.span.end)?;
                self.write(out, " => ");
                self.ty_node(out, &c.return_type.type_annotation)?;
            }
            TSType::TSConditionalType(c) => {
                self.ty_node(out, &c.check_type)?;
                self.write(out, " extends ");
                self.ty_node(out, &c.extends_type)?;
                self.write(out, " ? ");
                self.ty_node(out, &c.true_type)?;
                self.write(out, " : ");
                self.ty_node(out, &c.false_type)?;
            }
            TSType::TSInferType(i) => {
                self.write(out, "infer ");
                self.type_param_body(out, &i.type_parameter)?;
            }
            TSType::TSTypePredicate(p) => {
                if p.asserts {
                    // `emit(node.assertsModifier)` + `writeSpace`.
                    let start = skip_trivia(self.source, p.span.start as usize);
                    self.emit_node(out, self.pos_of(start as u32), (start + 7) as u32, |s, o| {
                        s.write(o, "asserts");
                        Some(())
                    })?;
                    self.write(out, " ");
                }
                let (span, text) = match &p.parameter_name {
                    TSTypePredicateName::Identifier(id) => (id.span, id.name.to_string()),
                    TSTypePredicateName::This(t) => (t.span, "this".to_string()),
                };
                self.emit_node(out, self.pos_of(span.start), span.end, |s, o| {
                    s.write(o, &text);
                    Some(())
                })?;
                if let Some(t) = &p.type_annotation {
                    self.write(out, " is ");
                    self.ty_node(out, &t.type_annotation)?;
                }
            }
            // TypeScript reports these as errors but prints them, with the
            // `?`/`!` in front even when it was written after the type.
            TSType::JSDocNullableType(t) => {
                self.write(out, "?");
                self.ty_node(out, &t.type_annotation)?;
            }
            TSType::JSDocNonNullableType(t) => {
                self.write(out, "!");
                self.ty_node(out, &t.type_annotation)?;
            }
            TSType::JSDocUnknownType(_) => self.write(out, "?"),
            // ngtsc throws "Unable to emit import type" on `import('...')`.
            TSType::TSImportType(_) => return None,
        }
        Some(())
    }

    /// `emitTokenWithComment`: the JSDoc leading comments at `pos` (skipped
    /// when the enclosing node starts there), the token, then its same-line
    /// JSDoc comments (skipped when the enclosing node ends at `context_end`
    /// — TypeScript compares against `contextNode.end`).
    fn token_with_comment(
        &mut self,
        out: &mut String,
        token: &'static str,
        pos: u32,
        end: u32,
        context_end: u32,
    ) -> Option<()> {
        self.emit_leading_of_pos(out, pos as usize);
        self.write(out, token);
        if end != context_end {
            self.emit_trailing_filtered(out, end as usize);
        }
        Some(())
    }

    /// `emitNamedTupleMember`: `[...]name[?]: type`, where the colon goes
    /// through `emitTokenWithComment` — which is how `lbl: /** c */ T` keeps
    /// its comment. `rest` is the `TSRestType` span when the member is
    /// `...name: type` (its `...` is `emit`ted, not punctuation).
    fn named_tuple_member_body(
        &mut self,
        out: &mut String,
        m: &TSNamedTupleMember<'a>,
        rest: Option<Span>,
    ) -> Option<()> {
        if let Some(rest) = rest {
            self.emit_node(out, self.pos_of(rest.start), rest.start + 3, |s, o| {
                s.write(o, "...");
                Some(())
            })?;
        }
        // `emit(node.name)`.
        self.emit_node(out, self.pos_of(m.label.span.start), m.label.span.end, |s, o| {
            s.write(o, &m.label.name);
            Some(())
        })?;
        if m.optional {
            // `emit(node.questionToken)`.
            let q = skip_trivia(self.source, m.label.span.end as usize);
            self.emit_node(out, self.pos_of(q as u32), q as u32 + 1, |s, o| {
                s.write(o, "?");
                Some(())
            })?;
        }
        // `emitTokenWithComment(ColonToken, node.name.end, ...)`: the `:` is
        // written where trivia after the name lands — on the `?` when
        // optional — and its trailing scan runs right after that position.
        let colon_end = skip_trivia(self.source, m.label.span.end as usize) + 1;
        self.token_with_comment(out, ":", m.label.span.end, colon_end as u32, m.span.end)?;
        self.write(out, " ");
        self.tuple_element_body(out, &m.element_type)
    }

    /// A tuple element (`emit` on `TSOptionalType`/`TSRestType`/`TSType`; a
    /// named member is a `TSType` variant).
    fn tuple_element_body(&mut self, out: &mut String, element: &TSTupleElement<'a>) -> Option<()> {
        match element {
            TSTupleElement::TSOptionalType(o) => {
                self.ty_node(out, &o.type_annotation)?;
                self.write(out, "?");
                Some(())
            }
            TSTupleElement::TSRestType(r) => {
                // A labeled rest (`...c: T[]`) is a named tuple member whose
                // `...` is an emitted token; an unlabeled one prints `...`
                // as punctuation (`emitRestOrJSDocVariadicType`).
                if let TSType::TSNamedTupleMember(m) = &r.type_annotation {
                    self.named_tuple_member_body(out, m, Some(r.span))
                } else {
                    self.write(out, "...");
                    self.ty_node(out, &r.type_annotation)
                }
            }
            other => self.ty_node(out, other.to_ts_type()),
        }
    }

    /// `emitLiteralType` → `emitExpression`: the literal as one node, so a
    /// `/** */` on its trailing edge prints.
    fn literal_body(&mut self, out: &mut String, literal: &TSLiteral<'a>) -> Option<()> {
        let (span, text) = match literal {
            TSLiteral::BooleanLiteral(b) => (b.span, b.value.to_string()),
            TSLiteral::NumericLiteral(n) => (n.span, format_number_like_js(n.value)),
            TSLiteral::BigIntLiteral(b) => (b.span, self.bigint(b)),
            TSLiteral::StringLiteral(s) => (s.span, quote_literal(s)),
            TSLiteral::TemplateLiteral(t) if t.expressions.is_empty() => {
                (t.span, self.slice(t.span))
            }
            TSLiteral::UnaryExpression(u) if u.operator == UnaryOperator::UnaryNegation => (
                u.span,
                match &u.argument {
                    Expression::NumericLiteral(n) => format!("-{}", format_number_like_js(n.value)),
                    Expression::BigIntLiteral(b) => format!("-{}", self.bigint(b)),
                    _ => return None,
                },
            ),
            _ => return None,
        };
        self.emit_node(out, self.pos_of(span.start), span.end, |s, o| {
            s.write(o, &text);
            Some(())
        })
    }

    /// TypeScript's scanner keeps hexadecimal bigints in hex (lowercased) and
    /// turns the other bases into decimal, without separators.
    fn bigint(&self, literal: &BigIntLiteral<'_>) -> String {
        let raw = self.slice(literal.span);
        match raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
            Some(hex) => format!("0x{}", hex.replace('_', "").to_ascii_lowercase()),
            None => format!("{}n", literal.value),
        }
    }

    /// `emitTypeArguments` / `emitTypeParameters`: a `<…>` list.
    fn type_args(
        &mut self,
        out: &mut String,
        args: &TSTypeParameterInstantiation<'a>,
    ) -> Option<()> {
        let items: Vec<El> = args.params.iter().map(El::Ty).collect();
        self.emit_list(
            out,
            &items,
            fmt::TYPE_ARGS,
            args.span.end,
            args.span.start as usize + 1,
            args.span.end as usize - 1,
            false,
            Some("<"),
            Some(">"),
        )
    }

    fn type_params(
        &mut self,
        out: &mut String,
        params: Option<&TSTypeParameterDeclaration<'a>>,
    ) -> Option<()> {
        let Some(params) = params else { return Some(()) };
        let items: Vec<El> = params.params.iter().map(El::TypeParam).collect();
        self.emit_list(
            out,
            &items,
            fmt::TYPE_ARGS,
            params.span.end,
            params.span.start as usize + 1,
            params.span.end as usize - 1,
            false,
            Some("<"),
            Some(">"),
        )
    }

    /// `emitTypeParameter`: modifiers, the name, `extends`, `=`.
    fn type_param_body(&mut self, out: &mut String, param: &TSTypeParameter<'a>) -> Option<()> {
        // The modifiers (`const`/`in`/`out`) are a `Modifiers` list in
        // TypeScript's AST; oxc gives flags, so the spans come from the text
        // in order.
        let mut mods: Vec<El> = Vec::new();
        let mut at = param.span.start as usize;
        for (set, word) in [(param.r#const, "const"), (param.r#in, "in"), (param.out, "out")] {
            if set {
                let start = skip_trivia(self.source, at);
                mods.push(El::Token {
                    span: Span::new(start as u32, start as u32 + word.len() as u32),
                    text: word,
                });
                at = start + word.len();
            }
        }
        if !mods.is_empty() {
            self.emit_list(out, &mods, fmt::MODIFIERS, param.span.end, at, at, false, None, None)?;
        }
        self.emit_node(out, self.pos_of(param.name.span.start), param.name.span.end, |s, o| {
            s.write(o, &param.name.name);
            Some(())
        })?;
        if let Some(constraint) = &param.constraint {
            self.write(out, " extends ");
            self.ty_node(out, constraint)?;
        }
        if let Some(default) = &param.default {
            self.write(out, " = ");
            self.ty_node(out, default)?;
        }
        Some(())
    }

    /// `emitParameters`: a `(…)` list of parameters.
    fn params(
        &mut self,
        out: &mut String,
        this: Option<&TSThisParameter<'a>>,
        params: &FormalParameters<'a>,
        parent_end: u32,
    ) -> Option<()> {
        let mut items: Vec<El> = Vec::new();
        if let Some(this) = this {
            items.push(El::This(this));
        }
        for param in &params.items {
            if param.initializer.is_some() {
                return None;
            }
            items.push(El::Param(param));
        }
        if let Some(rest) = &params.rest {
            items.push(El::Rest(rest));
        }
        self.emit_list(
            out,
            &items,
            fmt::PARAMETERS,
            parent_end,
            params.span.start as usize + 1,
            params.span.end as usize - 1,
            false,
            Some("("),
            Some(")"),
        )
    }

    /// `emitParameter`: modifiers, `...`, the name, `?`, `: type`.
    fn param_body(&mut self, out: &mut String, param: &FormalParameter<'a>) -> Option<()> {
        if param.readonly {
            let start = skip_trivia(self.source, param.span.start as usize);
            let mods =
                [El::Token { span: Span::new(start as u32, start as u32 + 8), text: "readonly" }];
            self.emit_list(
                out,
                &mods,
                fmt::MODIFIERS,
                param.span.end,
                start + 8,
                start + 8,
                false,
                None,
                None,
            )?;
        }
        self.emit_node(
            out,
            self.pos_of(param.pattern.span().start),
            param.pattern.span().end,
            |s, o| s.binding_body(o, &param.pattern),
        )?;
        if param.optional {
            let q = skip_trivia(self.source, param.pattern.span().end as usize);
            self.emit_node(out, self.pos_of(q as u32), q as u32 + 1, |s, o| {
                s.write(o, "?");
                Some(())
            })?;
        }
        self.annotation(out, param.type_annotation.as_deref())
    }

    /// The `this` parameter: `this` (`emitNodeWithWriter`) then `: type`.
    fn this_body(&mut self, out: &mut String, this: &TSThisParameter<'a>) -> Option<()> {
        self.emit_node(out, self.pos_of(this.this_span.start), this.this_span.end, |s, o| {
            s.write(o, "this");
            Some(())
        })?;
        self.annotation(out, this.type_annotation.as_deref())
    }

    /// A `FormalParameterRest` (`emitParameter` with a `...` token).
    fn rest_body(&mut self, out: &mut String, rest: &FormalParameterRest<'a>) -> Option<()> {
        // `emit(node.dotDotDotToken)`.
        self.emit_node(
            out,
            self.pos_of(rest.rest.span.start),
            rest.rest.span.start + 3,
            |s, o| {
                s.write(o, "...");
                Some(())
            },
        )?;
        self.emit_node(
            out,
            self.pos_of(rest.rest.argument.span().start),
            rest.rest.argument.span().end,
            |s, o| s.binding_body(o, &rest.rest.argument),
        )?;
        self.annotation(out, rest.type_annotation.as_deref())
    }

    /// An index signature's parameter (`emitParameter`): `name: type`. The
    /// name ends where `:` starts, so comments between them are trivia of
    /// the annotation — same as TypeScript's `Parameter`.
    fn index_param_body(
        &mut self,
        out: &mut String,
        param: &TSIndexSignatureName<'a>,
    ) -> Option<()> {
        let name_start = skip_trivia(self.source, param.span.start as usize);
        let name_end =
            token_end_before(&self.lexed, self.source, param.type_annotation.span.start as usize)
                .unwrap_or(param.type_annotation.span.start as usize);
        self.emit_node(out, self.pos_of(name_start as u32), name_end as u32, |s, o| {
            s.write(o, param.name.as_str());
            Some(())
        })?;
        self.annotation(out, Some(&param.type_annotation))
    }

    /// `emitParametersForIndexSignature`: a `[…]` list.
    fn index_params(
        &mut self,
        out: &mut String,
        param: &TSIndexSignatureName<'a>,
        parent_end: u32,
    ) -> Option<()> {
        let items = [El::IndexParam(param)];
        let list_pos = self.pos_of(param.span.start) as usize;
        self.emit_list(
            out,
            &items,
            fmt::INDEX_PARAMETERS,
            parent_end,
            list_pos,
            param.span.end as usize,
            false,
            Some("["),
            Some("]"),
        )
    }

    /// `: T`, or nothing — `emitTypeAnnotation`. The `:` is plain
    /// punctuation; the type keeps its own comments (a same-line one after
    /// `:` is trivia of the type and drops, like ngtsc).
    fn annotation(
        &mut self,
        out: &mut String,
        annotation: Option<&TSTypeAnnotation<'a>>,
    ) -> Option<()> {
        if let Some(a) = annotation {
            self.write(out, ": ");
            self.ty_node(out, &a.type_annotation)?;
        }
        Some(())
    }

    /// A member of a type literal (`emit<Kind>Signature`), ending in `;`.
    fn member_body(&mut self, out: &mut String, member: &TSSignature<'a>) -> Option<()> {
        match member {
            TSSignature::TSPropertySignature(p) => {
                if p.readonly {
                    self.modifiers(
                        out,
                        p.span.start,
                        p.span.end,
                        &[(p.span.start, 8, "readonly")],
                    )?;
                }
                self.key_node(out, &p.key, p.computed)?;
                if p.optional {
                    let q = skip_trivia(self.source, p.key.span().end as usize);
                    self.emit_node(out, self.pos_of(q as u32), q as u32 + 1, |s, o| {
                        s.write(o, "?");
                        Some(())
                    })?;
                }
                self.annotation(out, p.type_annotation.as_deref())?;
                self.write(out, ";");
            }
            TSSignature::TSIndexSignature(s) => {
                if s.readonly {
                    self.modifiers(
                        out,
                        s.span.start,
                        s.span.end,
                        &[(s.span.start, 8, "readonly")],
                    )?;
                }
                self.index_params(out, &s.parameter, s.span.end)?;
                self.annotation(out, Some(&s.type_annotation))?;
                self.write(out, ";");
            }
            TSSignature::TSCallSignatureDeclaration(c) => {
                self.type_params(out, c.type_parameters.as_deref())?;
                self.params(out, c.this_param.as_deref(), &c.params, c.span.end)?;
                self.annotation(out, c.return_type.as_deref())?;
                self.write(out, ";");
            }
            TSSignature::TSConstructSignatureDeclaration(c) => {
                self.write(out, "new ");
                self.type_params(out, c.type_parameters.as_deref())?;
                self.params(out, None, &c.params, c.span.end)?;
                self.annotation(out, c.return_type.as_deref())?;
                self.write(out, ";");
            }
            TSSignature::TSMethodSignature(m) => {
                // `get`/`set` are modifiers in TypeScript's AST.
                if m.kind != TSMethodSignatureKind::Method {
                    let (word, len) = match m.kind {
                        TSMethodSignatureKind::Get => ("get", 3),
                        TSMethodSignatureKind::Set => ("set", 3),
                        TSMethodSignatureKind::Method => unreachable!(),
                    };
                    self.modifiers(out, m.span.start, m.span.end, &[(m.span.start, len, word)])?;
                }
                self.key_node(out, &m.key, m.computed)?;
                if m.optional {
                    let q = skip_trivia(self.source, m.key.span().end as usize);
                    self.emit_node(out, self.pos_of(q as u32), q as u32 + 1, |s, o| {
                        s.write(o, "?");
                        Some(())
                    })?;
                }
                // `emitSignatureHead` + `emitEmptyFunctionBody`.
                self.type_params(out, m.type_parameters.as_deref())?;
                self.params(out, m.this_param.as_deref(), &m.params, m.span.end)?;
                self.annotation(out, m.return_type.as_deref())?;
                self.write(out, ";");
            }
        }
        Some(())
    }

    /// `emitModifierList` for keywords oxc exposes as flags. `from` is where
    /// the modifier text is searched for (the node's trivia end), `end` the
    /// owning node's `end`, and `mods` `(search-from, byte-len, text)` in
    /// order.
    fn modifiers(
        &mut self,
        out: &mut String,
        from: u32,
        end: u32,
        mods: &[(u32, u32, &'static str)],
    ) -> Option<()> {
        let items: Vec<El> = mods
            .iter()
            .map(|&(_, len, text)| {
                let start = skip_trivia(self.source, from as usize);
                El::Token { span: Span::new(start as u32, start as u32 + len), text }
            })
            .collect();
        self.emit_list(out, &items, fmt::MODIFIERS, end, 0, 0, false, None, None)
    }

    /// A property name as one node — `emitNodeWithWriter(node.name)` — or a
    /// computed one (`emitComputedPropertyName`): `[`, the expression, `]`.
    fn key_node(&mut self, out: &mut String, key: &PropertyKey<'a>, computed: bool) -> Option<()> {
        if computed {
            // `ComputedPropertyName`'s range: from the trivia before `[` to
            // `]`'s end. oxc's key span covers the expression only.
            let bracket_end = token_end_before(&self.lexed, self.source, key.span().start as usize)
                .unwrap_or(key.span().start as usize);
            let pos = token_end_before(&self.lexed, self.source, bracket_end - 1)
                .unwrap_or(bracket_end - 1);
            let end = skip_trivia(self.source, key.span().end as usize) + 1;
            let text = self.key_text(key)?;
            self.emit_node(out, pos as u32, end as u32, |s, o| {
                s.write(o, "[");
                s.emit_node(o, bracket_end as u32, key.span().end, |s2, o2| {
                    s2.write(o2, &text);
                    Some(())
                })?;
                s.write(o, "]");
                Some(())
            })
        } else {
            let pos = self.pos_of(key.span().start);
            let end = key.span().end;
            let text = self.key_text(key)?;
            self.emit_node(out, pos, end, |s, o| {
                s.write(o, &text);
                Some(())
            })
        }
    }

    /// A property name's text, re-quoted like any other string literal. A
    /// computed name (`[token]`, `[ns.token]`) follows the rules of a type
    /// name: an `@angular/core` value becomes `i0.token`, another module's
    /// makes the type `unknown`, and a local or global one stays as written
    /// (ngtsc copies the expression, which doesn't resolve for an import).
    ///
    /// Through an import-equals alias too, although ngtsc copies those as
    /// written: an alias of `@angular/core` gives `i0.token` and one of
    /// another module `unknown`, where ngtsc's name doesn't resolve in the
    /// `.d.ts`. An alias of a global is written as its target
    /// (`[Symbol.iterator]` for `import S = Symbol`), and one of a name the
    /// file declares stays as written, like ngtsc.
    fn key_text(&mut self, key: &PropertyKey<'a>) -> Option<String> {
        Some(match key {
            PropertyKey::StaticIdentifier(id) => id.name.to_string(),
            PropertyKey::StringLiteral(s) => quote_literal(s),
            PropertyKey::NumericLiteral(n) => format_number_like_js(n.value),
            PropertyKey::TemplateLiteral(t) if t.expressions.is_empty() => self.slice(t.span),
            PropertyKey::Identifier(id) => self.computed_name(std::vec![id.name.as_str()])?,
            PropertyKey::StaticMemberExpression(m) => {
                let mut parts = std::vec::Vec::new();
                member_parts(&m.object, &mut parts)?;
                parts.push(m.property.name.as_str());
                self.computed_name(parts)?
            }
            _ => return None,
        })
    }

    /// A computed property name `a.b.c` (`parts`, head first), as
    /// [`Self::key_text`] writes it.
    fn computed_name(&mut self, parts: std::vec::Vec<&str>) -> Option<String> {
        let written = parts.join(".");
        Some(match self.resolve_aliases(parts)? {
            Resolved::Core(rest) => format!("{}{rest}", self.core_ns),
            Resolved::OtherModule(_) => {
                self.other_module = true;
                written
            }
            Resolved::Name(parts, aliased) => {
                let (head, members) = (parts[0], &parts[1..]);
                if aliased && self.scope.import(head).is_none() && self.scope.declares(head) {
                    written
                } else {
                    let rest: String = members.iter().map(|m| format!(".{m}")).collect();
                    self.value_name(head, &rest)
                }
            }
        })
    }

    /// A `BindingPattern` node (`emit(name)` for a parameter's name).
    fn binding_body(&mut self, out: &mut String, pattern: &BindingPattern<'a>) -> Option<()> {
        match pattern {
            BindingPattern::BindingIdentifier(id) => self.write(out, &id.name),
            BindingPattern::ObjectPattern(o) => {
                self.write(out, "{");
                let mut items: Vec<El> = o.properties.iter().map(El::BindingProp).collect();
                if let Some(rest) = &o.rest {
                    items.push(El::BindingRest(rest));
                }
                // `ObjectBindingPatternElements` allows a trailing comma:
                // `{a,}` prints `{ a, }`.
                let trailing = items
                    .last()
                    .is_some_and(|el| self.trailing_comma_after(el.span().end as usize));
                self.emit_list(
                    out,
                    &items,
                    fmt::OBJECT_BINDING,
                    o.span.end,
                    o.span.start as usize + 1,
                    o.span.end as usize - 1,
                    trailing,
                    None,
                    None,
                )?;
                self.write(out, "}");
            }
            BindingPattern::ArrayPattern(a) => {
                self.write(out, "[");
                let mut items: Vec<El> = Vec::new();
                // Each element ends at a comma (or `]`). An elision is an
                // `OmittedExpression`: zero width right after the comma that
                // precedes it — trivia before that comma belongs to the
                // previous element, trivia after it is the elision's own.
                let mut cursor = a.span.start as usize + 1;
                let mut trailing = false;
                for element in &a.elements {
                    match element {
                        Some(element) => {
                            items.push(El::Binding(element));
                            cursor = element.span().end as usize;
                        }
                        None => {
                            items.push(El::Token {
                                span: Span::new(cursor as u32, cursor as u32),
                                text: "",
                            });
                        }
                    }
                    let after = skip_trivia(self.source, cursor);
                    trailing = self.source[after..].starts_with(',');
                    cursor = after + 1;
                }
                if let Some(rest) = &a.rest {
                    items.push(El::BindingRest(rest));
                    trailing = self.trailing_comma_after(rest.span.end as usize);
                }
                self.emit_list(
                    out,
                    &items,
                    fmt::ARRAY_BINDING,
                    a.span.end,
                    a.span.start as usize + 1,
                    a.span.end as usize - 1,
                    trailing,
                    None,
                    None,
                )?;
                self.write(out, "]");
            }
            // Default values aren't allowed in a type.
            BindingPattern::AssignmentPattern(_) => return None,
        }
        Some(())
    }

    /// A `BindingElement`: `propertyName: name`, or just `name` for
    /// shorthand (`emitBindingElement`; initializers can't appear in a type).
    fn binding_prop_body(
        &mut self,
        out: &mut String,
        property: &BindingProperty<'a>,
    ) -> Option<()> {
        if !property.shorthand {
            self.key_node(out, &property.key, property.computed)?;
            self.write(out, ": ");
        }
        self.emit_node(
            out,
            self.pos_of(property.value.span().start),
            property.value.span().end,
            |s, o| s.binding_body(o, &property.value),
        )
    }

    /// A `RestElement`: `...` then its argument (`emitRestElement` emits the
    /// `dotDotDotToken` with its comments).
    fn binding_rest_body(&mut self, out: &mut String, rest: &BindingRestElement<'a>) -> Option<()> {
        self.token_with_comment(out, "...", rest.span.start, rest.span.start + 3, rest.span.end)?;
        self.emit_node(
            out,
            self.pos_of(rest.argument.span().start),
            rest.argument.span().end,
            |s, o| s.binding_body(o, &rest.argument),
        )
    }

    /// `emitEntityName` for a `typeof` operand: each part is `emit`ted (so
    /// comments between parts survive), `.` between them.
    fn emit_query_name(&mut self, out: &mut String, name: &TSTypeQueryExprName<'a>) -> Option<()> {
        match name {
            TSTypeQueryExprName::IdentifierReference(id) => {
                self.emit_node(out, self.pos_of(id.span.start), id.span.end, |s, o| {
                    s.write(o, id.name.as_str());
                    Some(())
                })
            }
            TSTypeQueryExprName::QualifiedName(q) => {
                self.emit_name_left(out, &q.left)?;
                self.write(out, ".");
                self.emit_node(out, self.pos_of(q.right.span.start), q.right.span.end, |s, o| {
                    s.write(o, q.right.name.as_str());
                    Some(())
                })
            }
            TSTypeQueryExprName::ThisExpression(e) => {
                self.emit_node(out, self.pos_of(e.span.start), e.span.end, |s, o| {
                    s.write(o, "this");
                    Some(())
                })
            }
            TSTypeQueryExprName::TSImportType(_) => None,
        }
    }

    /// `emitEntityName`'s `left` recursion.
    fn emit_name_left(&mut self, out: &mut String, name: &TSTypeName<'a>) -> Option<()> {
        match name {
            TSTypeName::IdentifierReference(id) => {
                self.emit_node(out, self.pos_of(id.span.start), id.span.end, |s, o| {
                    s.write(o, id.name.as_str());
                    Some(())
                })
            }
            TSTypeName::QualifiedName(q) => {
                self.emit_name_left(out, &q.left)?;
                self.write(out, ".");
                self.emit_node(out, self.pos_of(q.right.span.start), q.right.span.end, |s, o| {
                    s.write(o, q.right.name.as_str());
                    Some(())
                })
            }
            TSTypeName::ThisExpression(e) => {
                self.emit_node(out, self.pos_of(e.span.start), e.span.end, |s, o| {
                    s.write(o, "this");
                    Some(())
                })
            }
        }
    }

    /// `@angular/core` names become `i0.Name`; local and global names stay as
    /// written; names from other modules set `other_module`.
    ///
    /// A qualified name whose head the file declares (`NS.T`, `C.T` for a
    /// class merged with a namespace, `A.B.T`, `E.T` for an enum merged with
    /// one) is written as its last part, `T`: ngtsc emits the declaration `T`
    /// resolves to by its own name. An enum member (`E.A`, `NS.E.A`) stays as
    /// written, since ngtsc can't emit one at all.
    ///
    /// An import-equals alias (`import A = NS`, `import C = NS.T`, or one
    /// declared in a namespace, `NS.A`) stands for its target, as ngtsc
    /// resolves it: `A.T`, `C` and `NS.A` are written as the name of the
    /// declaration they resolve to. An alias of another module
    /// (`import R = require('m')`, or of an import) is that module's, and one
    /// of `@angular/core` gives `i0.X`; ngtsc writes the bare name there,
    /// which doesn't resolve.
    fn type_name(&mut self, name: &TSTypeName<'_>) -> Option<String> {
        let mut parts = std::vec::Vec::new();
        entity_parts(name, &mut parts)?;
        Some(match self.resolve_aliases(parts)? {
            Resolved::Core(rest) => format!("{}{rest}", self.core_ns),
            Resolved::OtherModule(parts) => {
                self.other_module = true;
                parts.join(".")
            }
            Resolved::Name(parts, _) => self.resolved_name(&parts),
        })
    }

    /// `parts` (a name, head first) with the import-equals aliases it starts
    /// with replaced by their targets, until its head isn't one.
    fn resolve_aliases<'p>(&self, mut parts: std::vec::Vec<&'p str>) -> Option<Resolved<'p>>
    where
        'a: 'p,
    {
        let mut aliased = false;
        // Aliases can name aliases, through any number of them. Meeting one
        // again is a cycle (`import A = B; import B = A;`, `import A = A.B;`),
        // which TypeScript reports.
        let mut seen: std::vec::Vec<std::vec::Vec<&'p str>> = std::vec::Vec::new();
        loop {
            let (len, target) = match self.scope.alias(parts[0]) {
                Some(target) => (1, target),
                None => match self.scope.namespace_alias(&parts) {
                    Some(found) => found,
                    None => return Some(Resolved::Name(parts, aliased)),
                },
            };
            if seen.iter().any(|alias| alias[..] == parts[..len]) {
                return None;
            }
            seen.push(parts[..len].to_vec());
            aliased = true;
            match target {
                // `import Core = require('@angular/core')`: `Core.X` is
                // `i0.X`, like a namespace import's member.
                AliasTarget::Module(CORE) => {
                    return Some(Resolved::Core(
                        parts[len..].iter().map(|m| format!(".{m}")).collect(),
                    ));
                }
                AliasTarget::Module(_) => return Some(Resolved::OtherModule(parts)),
                AliasTarget::Entity(target) => {
                    parts.splice(0..len, target);
                }
            }
        }
    }

    /// [`Self::type_name`] for a name whose head isn't an alias: `parts`, head
    /// first.
    fn resolved_name(&mut self, parts: &[&str]) -> String {
        let (head, members) = (parts[0], &parts[1..]);
        if let [qualifier @ .., last] = parts
            && !members.is_empty()
            && self.scope.import(head).is_none()
            && self.scope.declares(head)
            && !self.scope.is_enum_member(qualifier, last)
        {
            return (*last).to_string();
        }
        let rest: String = members.iter().map(|m| format!(".{m}")).collect();
        self.value_name(head, &rest)
    }

    /// `head` followed by the `.member`s `rest`, with `head` resolved through
    /// the file's imports: `i0` for `@angular/core`, `other_module` for any
    /// other module, as written otherwise.
    fn value_name(&mut self, head: &str, rest: &str) -> String {
        match self.scope.import(head) {
            Some(import) if import.module == CORE => match import.imported {
                Some(imported) => format!("{}.{imported}{rest}", self.core_ns),
                None => format!("{}{rest}", self.core_ns),
            },
            Some(_) => {
                self.other_module = true;
                format!("{head}{rest}")
            }
            None => format!("{head}{rest}"),
        }
    }

    // ---- Comment emission (TypeScript's `onlyPrintJsDocStyle` paths) ----

    /// `emitLeadingCommentsOfPosition(pos)` — the JSDoc-only leading scan,
    /// gated on `containerPos` (`forEachLeadingCommentToEmit`).
    fn emit_leading_of_pos(&self, out: &mut String, pos: usize) {
        if pos as u32 == self.container_pos {
            return;
        }
        self.emit_leading(out, pos);
    }

    /// `emitLeadingComments(pos)` — comments after the first line break in
    /// the trivia at `pos`, JSDoc-style only. The first one gets a preceding
    /// newline when it starts on a different line than `pos`
    /// (`emitNewLineBeforeLeadingCommentOfPosition`), and each is followed by
    /// a newline when a line break follows it (`hasTrailingNewLine`), else a
    /// space for `/* */`.
    fn emit_leading(&self, out: &mut String, pos: usize) {
        let mut first = true;
        for c in comment_ranges(self.source, pos, false) {
            if !self.is_jsdoc_comment(c.pos) {
                continue;
            }
            if first && line_of(self.source, c.pos) != line_of(self.source, pos) {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            first = false;
            self.emit_comment_text(out, &c);
            if c.trailing_newline {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            } else if !c.line {
                out.push(' ');
            }
        }
    }

    /// `emitTrailingComments(end)` — comments on `end`'s line, JSDoc-style
    /// only, each preceded by a space unless at a line start; gated on
    /// `containerEnd` like `forEachTrailingCommentToEmit`.
    fn emit_trailing(&self, out: &mut String, end: usize) {
        if end as u32 == self.container_end {
            return;
        }
        for c in comment_ranges(self.source, end, true) {
            if !self.is_jsdoc_comment(c.pos) {
                continue;
            }
            if !out.is_empty() && !out.ends_with('\n') {
                out.push(' ');
            }
            self.emit_comment_text(out, &c);
            if c.trailing_newline && !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }

    /// `emitTrailingCommentsOfPosition(pos, prefixSpace: true)` — the
    /// JSDoc-filtered same-line scan `emitTokenWithComment` and the
    /// empty-brackets path use. Ungated (the position paths don't check
    /// `containerEnd`).
    fn emit_trailing_filtered(&self, out: &mut String, pos: usize) {
        for c in comment_ranges(self.source, pos, true) {
            if !self.is_jsdoc_comment(c.pos) {
                continue;
            }
            if !out.is_empty() && !out.ends_with('\n') {
                out.push(' ');
            }
            self.emit_comment_text(out, &c);
            if c.trailing_newline && !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }

    /// `emitTrailingCommentsOfPosition(pos)` with the default callback —
    /// comments on `pos`'s line, unfiltered, each followed by a space (or a
    /// newline for `hasTrailingNewLine`, which `//` and U+2028-following
    /// `/* */` have). This is what prints `/* */` comments the JSDoc-filtered
    /// paths drop — and prints them again for each enclosing list.
    fn emit_intervening(&self, out: &mut String, pos: usize) {
        for c in comment_ranges(self.source, pos, true) {
            self.emit_comment_text(out, &c);
            if c.trailing_newline {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            } else {
                out.push(' ');
            }
        }
    }

    /// The declaration printer's `shouldWriteComment`: `/**`-style or `/*!`
    /// (pinned) comments only — `/*` and `//` are dropped on node-level
    /// leading/trailing paths.
    fn is_jsdoc_comment(&self, pos: usize) -> bool {
        let b = self.source.as_bytes();
        b.get(pos + 1) == Some(&b'*')
            && (b.get(pos + 2) == Some(&b'!')
                || (b.get(pos + 2) == Some(&b'*') && b.get(pos + 3) != Some(&b'/')))
    }

    /// `writeCommentRange`: a `//` comment verbatim; a `/* */` with each line
    /// trimmed, continuation lines re-indented relative to the line the
    /// comment starts on, rebased to the writer's indent.
    fn emit_comment_text(&self, out: &mut String, comment: &Comment) {
        // `writeComment` goes through `writeText`, which indents a line
        // started by a `writeLine`.
        if out.ends_with('\n') {
            for _ in 0..self.indent {
                out.push_str(INDENT);
            }
        }
        let text = &self.source[comment.pos..comment.end];
        if comment.line {
            out.push_str(text);
            return;
        }
        let Some(first_break) = text.find(is_line_break) else {
            out.push_str(text);
            return;
        };
        // The first line, trimmed; `writeTrimmedCurrentLine` writes a
        // `writeLine` after it when the comment continues.
        out.push_str(text[..first_break].trim());
        out.push('\n');
        // `calculateIndent` of the line the comment starts on (the whitespace
        // before `/*`), then each continuation line's indent minus that,
        // rebased to the writer's indent.
        let first_indent =
            indent_of(&self.source[line_start_of(self.source, comment.pos)..comment.pos]);
        let mut rest = &text[first_break..];
        loop {
            rest = &rest[line_break_width(rest)..];
            let (line, next) = match rest.find(is_line_break) {
                Some(i) => (&rest[..i], &rest[i..]),
                None => (rest, ""),
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                // `writeTrimmedCurrentLine` writes a bare newline for an
                // all-whitespace continuation.
                out.push('\n');
            } else {
                let spaces =
                    4 * self.indent as isize - first_indent as isize + indent_of(line) as isize;
                for _ in 0..spaces.max(0) {
                    out.push(' ');
                }
                out.push_str(trimmed);
                if !next.is_empty() {
                    out.push('\n');
                }
            }
            if next.is_empty() {
                break;
            }
            rest = next;
        }
    }

    /// `NodeArray.hasTrailingComma`: a `,` between the last element's end
    /// and the closing token.
    fn trailing_comma_after(&self, end: usize) -> bool {
        self.source[skip_trivia(self.source, end)..].starts_with(',')
    }

    fn slice(&self, span: Span) -> String {
        span.source_text(self.source).to_string()
    }
}

/// A type name's parts, head first (`["A", "B", "T"]` for `A.B.T`). `None`
/// for one starting with `this`.
fn entity_parts<'a, 'n>(name: &'n TSTypeName<'a>, out: &mut Vec<&'n str>) -> Option<()> {
    match name {
        TSTypeName::IdentifierReference(id) => out.push(id.name.as_str()),
        TSTypeName::QualifiedName(q) => {
            entity_parts(&q.left, out)?;
            out.push(q.right.name.as_str());
        }
        TSTypeName::ThisExpression(_) => return None,
    }
    Some(())
}

/// The parts of a computed name's object, `a.b` in `[a.b.c]`, head first.
fn member_parts<'n>(object: &'n Expression<'_>, out: &mut Vec<&'n str>) -> Option<()> {
    match object {
        Expression::Identifier(id) => out.push(id.name.as_str()),
        Expression::StaticMemberExpression(m) if !m.optional => {
            member_parts(&m.object, out)?;
            out.push(m.property.name.as_str());
        }
        _ => return None,
    }
    Some(())
}

/// A name with its import-equals aliases replaced by their targets.
enum Resolved<'p> {
    /// An alias of `@angular/core`: the `.member`s after it.
    Core(String),
    /// An alias of another module: the name as written.
    OtherModule(std::vec::Vec<&'p str>),
    /// Any other name, head first, and whether an alias was replaced.
    Name(std::vec::Vec<&'p str>, bool),
}

/// TypeScript's `isLineBreak`: `\r`, `\n`, U+2028, U+2029.
fn is_line_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// TypeScript's `isWhiteSpaceSingleLine`: a fixed list — space, tab, VT, FF,
/// NBSP, NEL, ogham, the U+2000–U+200B range (enQuad through zeroWidthSpace),
/// narrow NBSP, mathematical space, ideographic space and the BOM. Rust's
/// `is_whitespace` differs on both ends (it lacks NEL and ZWSP and has
/// U+001C–U+001F), so the list is spelled out.
fn is_space(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\u{B}' | '\u{C}' | '\u{A0}' | '\u{85}' | '\u{1680}' | '\u{2000}'
            ..='\u{200B}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// `skipTrivia`: the position of the first token character at or after
/// `pos`, past whitespace and comments.
fn skip_trivia(source: &str, mut pos: usize) -> usize {
    loop {
        let Some(c) = char_at(source, pos) else { return pos };
        if is_whitespace_like(c) {
            pos += c.len_utf8();
        } else if source[pos..].starts_with("/*") {
            match source[pos + 2..].find("*/") {
                Some(end) => pos += 2 + end + 2,
                None => return source.len(),
            }
        } else if source[pos..].starts_with("//") {
            match source[pos..].find(is_line_break) {
                Some(end) => pos += end,
                None => return source.len(),
            }
        } else {
            return pos;
        }
    }
}

/// Where the token before `start` ends, skipping whitespace and comments
/// backwards. A `//` on the line above makes that line end at the `//` —
/// anything after it is comment — so the token ends at the `//` (or earlier,
/// on the line before it). `//` inside a string or a `/* */` doesn't count,
/// and the `/*` a trailing `*/` pairs with is the first one after the
/// previous `*/`, not the last.
fn token_end_before(lexed: &Lexed, source: &str, start: usize) -> Option<usize> {
    let mut end = start;
    loop {
        let before = &source[..end];
        // TypeScript's trivia, not Rust's Unicode whitespace: the trim must
        // take `isWhiteSpaceSingleLine` bytes like U+200B/U+FEFF and leave
        // the line breaks (`\u{2028}` included — a `//` after one still
        // counts for the same check above).
        let trimmed_len = before.trim_end_matches(is_whitespace_like).len();
        if before[trimmed_len..].contains(is_line_break) {
            if let Some(slashes) = line_comment_start(lexed, &before[..trimmed_len]) {
                // The line ends at the comment; keep looking before it.
                end = slashes;
                continue;
            }
        }
        end = trimmed_len;
        let trimmed = &before[..trimmed_len];
        match trimmed.strip_suffix("*/") {
            Some(inner) => end = block_comment_start(lexed, inner)?,
            None => return Some(end),
        }
    }
}

/// The source's comments: sorted `(start, end)` ranges of every `//` and
/// `/* */`, with delimiters — `end` of a `//` is the line break (or the
/// file's end), of `/* */` past the closer. Built from the parser's own
/// comment list, so strings, template text, and regex literals are already
/// skipped: a `/[/*]/` — after `=`, after `return`, after `)`, anywhere —
/// can't pair its `/*` with a comment's `*/` inside the type, and the
/// whole file is lexed once rather than re-scanned per `pos_of` call.
#[derive(Default)]
pub(crate) struct Lexed {
    /// `//` comment ranges, sorted by start.
    line: Vec<(usize, usize)>,
    /// `/* */` comment ranges, sorted by start.
    block: Vec<(usize, usize)>,
}

impl Lexed {
    /// Builds the table from the parser's comment list for one source file.
    pub(crate) fn from_comments(comments: &[OxcComment]) -> Self {
        let mut out = Self::default();
        for c in comments {
            let range = (c.span.start as usize, c.span.end as usize);
            match c.kind {
                CommentKind::Line => out.line.push(range),
                _ => out.block.push(range),
            }
        }
        out.line.sort_unstable();
        out.block.sort_unstable();
        out
    }
}

/// The `//` opening a line comment that runs to the end of `code` — so it
/// sits on the last line and bounds the trivia — or `None`. The ranges are
/// sorted and disjoint, so the only `//` that can cover `code.len()` is the
/// last one starting before it.
fn line_comment_start(lexed: &Lexed, code: &str) -> Option<usize> {
    let i = lexed.line.partition_point(|(start, _)| *start < code.len());
    match lexed.line.get(i.wrapping_sub(1)) {
        Some(&(start, end)) if i > 0 && end >= code.len() => Some(start),
        _ => None,
    }
}

/// The `/*` that opens the comment closed by the `*/` just cut off `inner`:
/// the `/*` whose own `*/` isn't inside `inner` (a `/*` with one is a
/// comment of its own — or comment text — and is skipped whole). Same
/// binary-search shape as [`line_comment_start`].
fn block_comment_start(lexed: &Lexed, inner: &str) -> Option<usize> {
    let i = lexed.block.partition_point(|(start, _)| *start < inner.len());
    match lexed.block.get(i.wrapping_sub(1)) {
        Some(&(start, end)) if i > 0 && end > inner.len() => Some(start),
        _ => None,
    }
}
/// A comment's source range, as TypeScript's `iterateCommentRanges` reports
/// it.
struct Comment {
    pos: usize,
    end: usize,
    /// `//`: the comment ends at a line break.
    line: bool,
    /// A line break follows the comment (`hasTrailingNewLine`): always true
    /// for `//`, true for `/* */` when a `\r`, `\n`, U+2028 or U+2029
    /// follows while the scan collects it.
    trailing_newline: bool,
}

/// `getLeadingCommentRanges` / `getTrailingCommentRanges` (`scanner.ts`
/// `iterateCommentRanges`): the comments in the trivia starting at `pos`.
///
/// A trailing scan collects comments up to the first `\r`/`\n` — those on
/// `pos`'s line. A leading scan skips that same-line prefix and collects
/// everything after it. The quirk ngtsc's doubled comments ride on:
/// U+2028/U+2029 are whitespace here (a trailing scan crosses them, and they
/// don't start collecting for a leading one), while the line map counts them
/// as line breaks.
fn comment_ranges(source: &str, start: usize, trailing: bool) -> Vec<Comment> {
    let mut out = Vec::new();
    // Pending = a comment seen while collecting; flushed when the next
    // comment starts or the scan ends.
    let mut pending: Option<Comment> = None;
    let mut collecting = trailing || start == 0;
    let bytes = source.as_bytes();
    let mut pos = start;
    while pos < bytes.len() {
        let b = bytes[pos];
        match b {
            b'\r' | b'\n' => {
                if b == b'\r' && bytes.get(pos + 1) == Some(&b'\n') {
                    pos += 1;
                }
                pos += 1;
                if trailing {
                    break;
                }
                collecting = true;
                if let Some(c) = &mut pending {
                    c.trailing_newline = true;
                }
            }
            b' ' | b'\t' | 0x0B | 0x0C => pos += 1,
            b'/' => {
                let next = bytes.get(pos + 1);
                let is_line = next == Some(&b'/');
                if next != Some(&b'/') && next != Some(&b'*') {
                    break;
                }
                let comment_start = pos;
                pos += 2;
                let mut trailing_newline = false;
                if is_line {
                    while let Some(c) = char_at(source, pos) {
                        if is_line_break(c) {
                            trailing_newline = true;
                            break;
                        }
                        pos += c.len_utf8();
                    }
                } else {
                    while pos < bytes.len() {
                        if bytes[pos] == b'*' && bytes.get(pos + 1) == Some(&b'/') {
                            pos += 2;
                            break;
                        }
                        pos += 1;
                    }
                }
                if collecting {
                    if let Some(c) = pending.take() {
                        out.push(c);
                    }
                    pending = Some(Comment {
                        pos: comment_start,
                        end: pos,
                        line: is_line,
                        trailing_newline,
                    });
                }
            }
            _ => {
                // Non-ASCII whitespace — including U+2028/U+2029, which are
                // whitespace here (a trailing scan crosses them; they don't
                // start a leading scan collecting) while the line map still
                // counts them as line breaks.
                match char_at(source, pos) {
                    Some(c) if (c as u32) > 0x7F && is_whitespace_like(c) => {
                        if is_line_break(c)
                            && let Some(p) = &mut pending
                        {
                            p.trailing_newline = true;
                        }
                        pos += c.len_utf8();
                    }
                    _ => break,
                }
            }
        }
    }
    if let Some(c) = pending {
        out.push(c);
    }
    out
}

/// TypeScript's `isWhiteSpaceLike`: single-line whitespace or a line break.
fn is_whitespace_like(c: char) -> bool {
    is_space(c) || is_line_break(c)
}

/// The character at byte `pos`, or `None` mid-character/past the end.
fn char_at(source: &str, pos: usize) -> Option<char> {
    source.get(pos..)?.chars().next()
}

/// The 1-based-agnostic line index of `pos` (number of line breaks before
/// it), where U+2028/U+2029 count — matching TypeScript's line map.
fn line_of(source: &str, pos: usize) -> usize {
    source[..pos].chars().filter(|&c| is_line_break(c)).count()
}

/// Byte index where `pos`'s line starts (just past the last line break).
fn line_start_of(source: &str, pos: usize) -> usize {
    source[..pos]
        .char_indices()
        .rfind(|&(_, c)| is_line_break(c))
        .map_or(0, |(i, c)| i + c.len_utf8())
}

/// `calculateIndent`: the width of `text`'s leading whitespace, tabs rounded
/// to the next multiple of 4.
fn indent_of(text: &str) -> usize {
    let mut indent = 0;
    for c in text.chars() {
        if c == '\t' {
            indent += 4 - indent % 4;
        } else if is_space(c) {
            indent += 1;
        } else {
            break;
        }
    }
    indent
}

/// Width in bytes of the line break `text` starts with.
fn line_break_width(text: &str) -> usize {
    match text.as_bytes().first() {
        Some(b'\r') if text.as_bytes().get(1) == Some(&b'\n') => 2,
        Some(b'\r') | Some(b'\n') => 1,
        _ => text.chars().next().map_or(0, char::len_utf8),
    }
}

/// A string literal as TypeScript prints a synthesized one: double quotes,
/// escapes for control characters and everything outside ASCII (as UTF-16
/// code units).
pub(crate) fn quote(s: &str) -> String {
    quote_units(s.encode_utf16())
}

/// [`quote`] for a string literal from the source, whose value can hold lone
/// surrogates (`'\uD800'`): the parser encodes each as `\u{FFFD}` and its
/// code unit in hex (and a `\u{FFFD}` as `\u{FFFD}fffd`).
fn quote_literal(literal: &StringLiteral<'_>) -> String {
    if !literal.lone_surrogates {
        return quote(&literal.value);
    }
    let mut units = Vec::new();
    let mut chars = literal.value.chars();
    while let Some(c) = chars.next() {
        if c == '\u{FFFD}' {
            let hex: String = chars.by_ref().take(4).collect();
            if let Ok(unit) = u16::from_str_radix(&hex, 16) {
                units.push(unit);
                continue;
            }
        }
        units.extend(c.encode_utf16(&mut [0; 2]).iter());
    }
    quote_units(units)
}

fn quote_units(units: impl IntoIterator<Item = u16>) -> String {
    let units: Vec<u16> = units.into_iter().collect();
    let mut out = String::with_capacity(units.len() + 2);
    out.push('"');
    for (i, &unit) in units.iter().enumerate() {
        match unit {
            0x22 => out.push_str("\\\""),
            0x5c => out.push_str("\\\\"),
            0x0a => out.push_str("\\n"),
            0x0d => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            0x0b => out.push_str("\\v"),
            0x0c => out.push_str("\\f"),
            0x08 => out.push_str("\\b"),
            // `\0` followed by a digit would read as an octal escape.
            0 if units.get(i + 1).is_some_and(|u| (0x30..=0x39).contains(u)) => {
                out.push_str("\\x00");
            }
            0 => out.push_str("\\0"),
            unit if unit < 0x20 || unit > 0x7f => {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!("\\u{unit:04X}"));
            }
            unit => out.push(char::from(unit as u8)),
        }
    }
    out.push('"');
    out
}
