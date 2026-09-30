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

use oxc_ast::ast::{
    BigIntLiteral, BindingPattern, Expression, FormalParameters, PropertyKey, TSLiteral,
    TSMappedTypeModifierOperator, TSMethodSignatureKind, TSSignature, TSThisParameter,
    TSTupleElement, TSType, TSTypeAnnotation, TSTypeName, TSTypeOperatorOperator, TSTypeParameter,
    TSTypeParameterDeclaration, TSTypeParameterInstantiation, TSTypePredicateName,
    TSTypeQueryExprName, UnaryOperator,
};
use oxc_span::GetSpan;

use super::evaluator::FileScope;
use crate::output::emitter::format_number_like_js;

pub(crate) struct TypePrinter<'s, 'a> {
    pub scope: &'s FileScope<'a>,
    pub source: &'a str,
    /// Set when the type references a module other than `@angular/core`.
    /// ngtsc would add an `import * as iN` for it; oxc emits `unknown` instead,
    /// since aliases numbered per source file can't be merged into bundled
    /// declaration files safely.
    pub other_module: bool,
}

impl TypePrinter<'_, '_> {
    /// `unknown` when the type has a form ngtsc can't emit either (an
    /// `import('...')` type) or that isn't valid in a type annotation.
    pub(crate) fn print(&mut self, ty: &TSType<'_>) -> String {
        self.ty(ty).unwrap_or_else(|| "unknown".to_string())
    }

    fn ty(&mut self, ty: &TSType<'_>) -> Option<String> {
        Some(match ty {
            TSType::TSAnyKeyword(_) => "any".into(),
            TSType::TSBigIntKeyword(_) => "bigint".into(),
            TSType::TSBooleanKeyword(_) => "boolean".into(),
            TSType::TSIntrinsicKeyword(_) => "intrinsic".into(),
            TSType::TSNeverKeyword(_) => "never".into(),
            TSType::TSNullKeyword(_) => "null".into(),
            TSType::TSNumberKeyword(_) => "number".into(),
            TSType::TSObjectKeyword(_) => "object".into(),
            TSType::TSStringKeyword(_) => "string".into(),
            TSType::TSSymbolKeyword(_) => "symbol".into(),
            TSType::TSUndefinedKeyword(_) => "undefined".into(),
            TSType::TSUnknownKeyword(_) => "unknown".into(),
            TSType::TSVoidKeyword(_) => "void".into(),
            TSType::TSThisType(_) => "this".into(),
            TSType::TSTypeReference(r) => {
                let mut out = self.type_name(&r.type_name)?;
                if let Some(args) = &r.type_arguments {
                    out.push_str(&self.type_args(args)?);
                }
                out
            }
            TSType::TSUnionType(u) => self.constituents(&u.types, u.span.start, "|")?,
            TSType::TSIntersectionType(i) => self.constituents(&i.types, i.span.start, "&")?,
            TSType::TSParenthesizedType(p) => format!("({})", self.ty(&p.type_annotation)?),
            TSType::TSArrayType(a) => format!("{}[]", self.ty(&a.element_type)?),
            TSType::TSIndexedAccessType(i) => {
                format!("{}[{}]", self.ty(&i.object_type)?, self.ty(&i.index_type)?)
            }
            TSType::TSTypeOperatorType(o) => {
                let op = match o.operator {
                    TSTypeOperatorOperator::Keyof => "keyof",
                    TSTypeOperatorOperator::Unique => "unique",
                    TSTypeOperatorOperator::Readonly => "readonly",
                };
                format!("{op} {}", self.ty(&o.type_annotation)?)
            }
            TSType::TSTupleType(t) => {
                let items =
                    self.elements(Some(t.span.start), &t.element_types, Self::tuple_element)?;
                format!("[{}]", items.join(", "))
            }
            TSType::TSNamedTupleMember(m) => format!(
                "{}{}: {}",
                m.label.name,
                if m.optional { "?" } else { "" },
                self.tuple_element(&m.element_type)?
            ),
            TSType::TSLiteralType(l) => self.literal(&l.literal)?,
            TSType::TSTemplateLiteralType(t) => {
                let mut out = String::from("`");
                for (i, quasi) in t.quasis.iter().enumerate() {
                    out.push_str(quasi.value.raw.as_str());
                    if let Some(ty) = t.types.get(i) {
                        out.push_str("${");
                        out.push_str(&self.ty(ty)?);
                        out.push('}');
                    }
                }
                out.push('`');
                out
            }
            TSType::TSTypeQuery(q) => {
                // `typeof x` names a value; ngtsc leaves it as written.
                let mut out = match &q.expr_name {
                    TSTypeQueryExprName::IdentifierReference(id) => format!("typeof {}", id.name),
                    TSTypeQueryExprName::QualifiedName(name) => {
                        format!("typeof {}.{}", entity_name(&name.left)?, name.right.name)
                    }
                    _ => return None,
                };
                if let Some(args) = &q.type_arguments {
                    out.push_str(&self.type_args(args)?);
                }
                out
            }
            TSType::TSTypeLiteral(l) if l.members.is_empty() => "{}".into(),
            TSType::TSTypeLiteral(l) => {
                let members = self.elements(Some(l.span.start), &l.members, Self::member)?;
                format!("{{ {} }}", members.join(" "))
            }
            TSType::TSMappedType(m) => {
                let readonly = match m.readonly {
                    None => "",
                    Some(TSMappedTypeModifierOperator::True) => "readonly ",
                    Some(TSMappedTypeModifierOperator::Plus) => "+readonly ",
                    Some(TSMappedTypeModifierOperator::Minus) => "-readonly ",
                };
                let optional = match m.optional {
                    None => "",
                    Some(TSMappedTypeModifierOperator::True) => "?",
                    Some(TSMappedTypeModifierOperator::Plus) => "+?",
                    Some(TSMappedTypeModifierOperator::Minus) => "-?",
                };
                let name_type = match &m.name_type {
                    Some(t) => format!(" as {}", self.ty(t)?),
                    None => String::new(),
                };
                let value = match &m.type_annotation {
                    Some(t) => self.ty(t)?,
                    None => String::new(),
                };
                format!(
                    "{{ {readonly}[{} in {}{name_type}]{optional}: {value}; }}",
                    m.key.name,
                    self.ty(&m.constraint)?
                )
            }
            TSType::TSFunctionType(f) => format!(
                "{}{} => {}",
                self.type_params(f.type_parameters.as_deref())?,
                self.params(f.this_param.as_deref(), &f.params)?,
                self.ty(&f.return_type.type_annotation)?
            ),
            TSType::TSConstructorType(c) => format!(
                "{}new {}{} => {}",
                if c.r#abstract { "abstract " } else { "" },
                self.type_params(c.type_parameters.as_deref())?,
                self.params(None, &c.params)?,
                self.ty(&c.return_type.type_annotation)?
            ),
            TSType::TSConditionalType(c) => format!(
                "{} extends {} ? {} : {}",
                self.ty(&c.check_type)?,
                self.ty(&c.extends_type)?,
                self.ty(&c.true_type)?,
                self.ty(&c.false_type)?
            ),
            TSType::TSInferType(i) => format!("infer {}", self.type_param(&i.type_parameter)?),
            TSType::TSTypePredicate(p) => {
                let name = match &p.parameter_name {
                    TSTypePredicateName::Identifier(id) => id.name.as_str(),
                    TSTypePredicateName::This(_) => "this",
                };
                let mut out = if p.asserts { format!("asserts {name}") } else { name.to_string() };
                if let Some(t) = &p.type_annotation {
                    out.push_str(" is ");
                    out.push_str(&self.ty(&t.type_annotation)?);
                }
                out
            }
            // TypeScript reports these as errors but prints them, with the
            // `?`/`!` in front even when it was written after the type.
            TSType::JSDocNullableType(t) => format!("?{}", self.ty(&t.type_annotation)?),
            TSType::JSDocNonNullableType(t) => format!("!{}", self.ty(&t.type_annotation)?),
            TSType::JSDocUnknownType(_) => "?".into(),
            // ngtsc throws "Unable to emit import type" on `import('...')`.
            TSType::TSImportType(_) => return None,
        })
    }

    /// `@angular/core` names become `i0.Name`; local and global names stay as
    /// written; names from other modules set `other_module`.
    ///
    /// A qualified name whose head the file declares (`NS.T`, `C.T` for a
    /// class merged with a namespace, `A.B.T`) is written as its last part,
    /// `T`: ngtsc emits the declaration `T` resolves to by its own name. An enum
    /// member (`E.A`) stays as written, since ngtsc can't emit one at all.
    fn type_name(&mut self, name: &TSTypeName<'_>) -> Option<String> {
        if let TSTypeName::QualifiedName(q) = name {
            let mut left = &q.left;
            let mut depth = 1;
            while let TSTypeName::QualifiedName(inner) = left {
                left = &inner.left;
                depth += 1;
            }
            if let TSTypeName::IdentifierReference(head) = left
                && self.scope.import(head.name.as_str()).is_none()
                && self.scope.declares(head.name.as_str())
                && !(depth == 1 && self.scope.is_enum(head.name.as_str()))
            {
                return Some(q.right.name.to_string());
            }
        }
        let (head, rest) = match name {
            TSTypeName::IdentifierReference(id) => (id.name.as_str(), String::new()),
            TSTypeName::QualifiedName(q) => {
                let mut left = &q.left;
                let mut rest = format!(".{}", q.right.name);
                while let TSTypeName::QualifiedName(inner) = left {
                    rest = format!(".{}{rest}", inner.right.name);
                    left = &inner.left;
                }
                match left {
                    TSTypeName::IdentifierReference(id) => (id.name.as_str(), rest),
                    _ => return None,
                }
            }
            TSTypeName::ThisExpression(_) => return None,
        };
        Some(match self.scope.import(head) {
            Some(import) if import.module == "@angular/core" => match import.imported {
                Some(imported) => format!("i0.{imported}{rest}"),
                None => format!("i0{rest}"),
            },
            Some(_) => {
                self.other_module = true;
                format!("{head}{rest}")
            }
            None => format!("{head}{rest}"),
        })
    }

    /// A union's or intersection's constituents, joined by ` | ` / ` & `.
    fn constituents(&mut self, types: &[TSType<'_>], start: u32, op: &str) -> Option<String> {
        // After a leading `|`/`&`, the first constituent follows that token;
        // otherwise it shares the start of the union (see `leading_comments`).
        let open = self.source[start as usize..].starts_with(op).then_some(start);
        Some(self.elements(open, types, Self::ty)?.join(&format!(" {op} ")))
    }

    fn type_args(&mut self, args: &TSTypeParameterInstantiation<'_>) -> Option<String> {
        Some(format!(
            "<{}>",
            self.elements(Some(args.span.start), &args.params, Self::ty)?.join(", ")
        ))
    }

    fn type_params(&mut self, params: Option<&TSTypeParameterDeclaration<'_>>) -> Option<String> {
        let Some(params) = params else { return Some(String::new()) };
        let items = self.elements(Some(params.span.start), &params.params, Self::type_param)?;
        Some(format!("<{}>", items.join(", ")))
    }

    fn type_param(&mut self, param: &TSTypeParameter<'_>) -> Option<String> {
        let mut out = String::new();
        if param.r#const {
            out.push_str("const ");
        }
        if param.r#in {
            out.push_str("in ");
        }
        if param.out {
            out.push_str("out ");
        }
        out.push_str(&param.name.name);
        if let Some(constraint) = &param.constraint {
            out.push_str(" extends ");
            out.push_str(&self.ty(constraint)?);
        }
        if let Some(default) = &param.default {
            out.push_str(" = ");
            out.push_str(&self.ty(default)?);
        }
        Some(out)
    }

    fn tuple_element(&mut self, element: &TSTupleElement<'_>) -> Option<String> {
        match element {
            TSTupleElement::TSOptionalType(o) => Some(format!("{}?", self.ty(&o.type_annotation)?)),
            TSTupleElement::TSRestType(r) => Some(format!("...{}", self.ty(&r.type_annotation)?)),
            other => self.ty(other.to_ts_type()),
        }
    }

    fn literal(&self, literal: &TSLiteral<'_>) -> Option<String> {
        Some(match literal {
            TSLiteral::BooleanLiteral(b) => b.value.to_string(),
            TSLiteral::NumericLiteral(n) => format_number_like_js(n.value),
            TSLiteral::BigIntLiteral(b) => self.bigint(b),
            TSLiteral::StringLiteral(s) => quote(&s.value),
            TSLiteral::TemplateLiteral(t) if t.expressions.is_empty() => self.slice(t.span),
            TSLiteral::UnaryExpression(u) if u.operator == UnaryOperator::UnaryNegation => {
                match &u.argument {
                    Expression::NumericLiteral(n) => format!("-{}", format_number_like_js(n.value)),
                    Expression::BigIntLiteral(b) => format!("-{}", self.bigint(b)),
                    _ => return None,
                }
            }
            _ => return None,
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

    fn member(&mut self, member: &TSSignature<'_>) -> Option<String> {
        Some(match member {
            TSSignature::TSPropertySignature(p) => format!(
                "{}{}{}{};",
                if p.readonly { "readonly " } else { "" },
                self.key(&p.key, p.computed)?,
                if p.optional { "?" } else { "" },
                self.annotation(p.type_annotation.as_deref())?
            ),
            TSSignature::TSIndexSignature(s) => {
                let param = &s.parameter;
                format!(
                    "{}[{}{}: {}]: {};",
                    if s.readonly { "readonly " } else { "" },
                    self.leading_comments(None, param.span.start),
                    param.name,
                    self.ty(&param.type_annotation.type_annotation)?,
                    self.ty(&s.type_annotation.type_annotation)?
                )
            }
            TSSignature::TSCallSignatureDeclaration(c) => format!(
                "{}{}{};",
                self.type_params(c.type_parameters.as_deref())?,
                self.params(c.this_param.as_deref(), &c.params)?,
                self.annotation(c.return_type.as_deref())?
            ),
            TSSignature::TSConstructSignatureDeclaration(c) => format!(
                "new {}{}{};",
                self.type_params(c.type_parameters.as_deref())?,
                self.params(None, &c.params)?,
                self.annotation(c.return_type.as_deref())?
            ),
            TSSignature::TSMethodSignature(m) => format!(
                "{}{}{}{}{}{};",
                match m.kind {
                    TSMethodSignatureKind::Method => "",
                    TSMethodSignatureKind::Get => "get ",
                    TSMethodSignatureKind::Set => "set ",
                },
                self.key(&m.key, m.computed)?,
                if m.optional { "?" } else { "" },
                self.type_params(m.type_parameters.as_deref())?,
                self.params(m.this_param.as_deref(), &m.params)?,
                self.annotation(m.return_type.as_deref())?
            ),
        })
    }

    /// `: T`, or nothing when there's no annotation.
    fn annotation(&mut self, annotation: Option<&TSTypeAnnotation<'_>>) -> Option<String> {
        match annotation {
            Some(t) => Some(format!(": {}", self.ty(&t.type_annotation)?)),
            None => Some(String::new()),
        }
    }

    /// A property name, re-quoted like any other string literal.
    fn key(&self, key: &PropertyKey<'_>, computed: bool) -> Option<String> {
        let text = match key {
            PropertyKey::StaticIdentifier(id) => id.name.to_string(),
            PropertyKey::StringLiteral(s) => quote(&s.value),
            PropertyKey::NumericLiteral(n) => format_number_like_js(n.value),
            PropertyKey::TemplateLiteral(t) if t.expressions.is_empty() => self.slice(t.span),
            PropertyKey::Identifier(id) => id.name.to_string(),
            PropertyKey::StaticMemberExpression(m) => member_chain(&m.object, &m.property.name)?,
            _ => return None,
        };
        Some(if computed { format!("[{text}]") } else { text })
    }

    fn params(
        &mut self,
        this: Option<&TSThisParameter<'_>>,
        params: &FormalParameters<'_>,
    ) -> Option<String> {
        let mut out = Vec::new();
        let mut prev = params.span.start;
        if let Some(this) = this {
            let comments = self.leading_comments(Some(prev), this.span.start);
            out.push(format!(
                "{comments}this{}",
                self.annotation(this.type_annotation.as_deref())?
            ));
            prev = this.span.end;
        }
        for param in &params.items {
            if param.initializer.is_some() {
                return None;
            }
            let comments = self.leading_comments(Some(prev), param.span.start);
            out.push(format!(
                "{comments}{}{}{}",
                self.binding(&param.pattern)?,
                if param.optional { "?" } else { "" },
                self.annotation(param.type_annotation.as_deref())?
            ));
            prev = param.span.end;
        }
        if let Some(rest) = &params.rest {
            let comments = self.leading_comments(Some(prev), rest.span.start);
            out.push(format!(
                "{comments}...{}{}",
                self.binding(&rest.rest.argument)?,
                self.annotation(rest.type_annotation.as_deref())?
            ));
        }
        Some(format!("({})", out.join(", ")))
    }

    /// A parameter name or destructuring pattern.
    fn binding(&mut self, pattern: &BindingPattern<'_>) -> Option<String> {
        match pattern {
            BindingPattern::BindingIdentifier(id) => Some(id.name.to_string()),
            BindingPattern::ObjectPattern(o) if o.properties.is_empty() && o.rest.is_none() => {
                Some("{}".into())
            }
            BindingPattern::ObjectPattern(o) => {
                let mut items = Vec::new();
                let mut prev = o.span.start;
                for property in &o.properties {
                    let value = self.binding(&property.value)?;
                    let item = if property.shorthand {
                        value
                    } else {
                        format!("{}: {value}", self.key(&property.key, property.computed)?)
                    };
                    items.push(self.leading_comments(Some(prev), property.span.start) + &item);
                    prev = property.span.end;
                }
                if let Some(rest) = &o.rest {
                    let comments = self.leading_comments(Some(prev), rest.span.start);
                    items.push(format!("{comments}...{}", self.binding(&rest.argument)?));
                }
                Some(format!("{{ {} }}", items.join(", ")))
            }
            BindingPattern::ArrayPattern(a) => {
                let mut items = Vec::new();
                let mut prev = a.span.start;
                for element in &a.elements {
                    let Some(element) = element else {
                        items.push(String::new());
                        continue;
                    };
                    let comments = self.leading_comments(Some(prev), element.span().start);
                    items.push(comments + &self.binding(element)?);
                    prev = element.span().end;
                }
                if let Some(rest) = &a.rest {
                    let comments = self.leading_comments(Some(prev), rest.span.start);
                    items.push(format!("{comments}...{}", self.binding(&rest.argument)?));
                }
                Some(format!("[{}]", items.join(", ")))
            }
            // Default values aren't allowed in a type.
            BindingPattern::AssignmentPattern(_) => None,
        }
    }

    /// Prints each element of a list, with the comments TypeScript keeps in
    /// front of it. `open` is where the list's opening token (`<`, `(`, `{`,
    /// `[`, a leading `|`) starts; `None` when the list has none, so the token
    /// before the first element is looked for backwards.
    fn elements<T: GetSpan>(
        &mut self,
        open: Option<u32>,
        items: &[T],
        mut print: impl FnMut(&mut Self, &T) -> Option<String>,
    ) -> Option<Vec<String>> {
        let mut prev = open;
        items
            .iter()
            .map(|item| {
                let span = item.span();
                let comments = self.leading_comments(prev, span.start);
                prev = Some(span.end);
                Some(comments + &print(self, item)?)
            })
            .collect()
    }

    /// The comments TypeScript's printer keeps in front of a list element:
    /// those right after the token before it, on that token's line (it emits
    /// them as trailing comments of that position). Other comments in the type
    /// are dropped, as ngtsc does, except `/** */` ones after a type or on
    /// their own line, which ngtsc keeps and oxc doesn't.
    ///
    /// `prev` is where to look for that token from: the end of the previous
    /// element (the token is the delimiter, if any) or the opening bracket.
    /// Without it the token is searched for backwards from `start`, which is
    /// how a union's first constituent finds the token before the union.
    fn leading_comments(&self, prev: Option<u32>, start: u32) -> String {
        let start = start as usize;
        let pos = match prev {
            Some(prev) => after_token(self.source, prev as usize, start),
            None => match token_end_before(self.source, start) {
                Some(pos) => pos,
                None => return String::new(),
            },
        };
        same_line_comments(&self.source[pos..start])
    }

    fn slice(&self, span: oxc_span::Span) -> String {
        span.source_text(self.source).to_string()
    }
}

/// A `typeof` operand or computed key: `a.b.c`, as ngtsc prints it (without
/// the source's spacing).
fn entity_name(name: &TSTypeName<'_>) -> Option<String> {
    match name {
        TSTypeName::IdentifierReference(id) => Some(id.name.to_string()),
        TSTypeName::QualifiedName(q) => Some(format!("{}.{}", entity_name(&q.left)?, q.right.name)),
        TSTypeName::ThisExpression(_) => Some("this".into()),
    }
}

fn member_chain(object: &Expression<'_>, property: &str) -> Option<String> {
    let object = match object {
        Expression::Identifier(id) => id.name.to_string(),
        Expression::StaticMemberExpression(m) if !m.optional => {
            member_chain(&m.object, &m.property.name)?
        }
        _ => return None,
    };
    Some(format!("{object}.{property}"))
}

fn is_line_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn is_space(c: char) -> bool {
    c.is_whitespace() && !is_line_break(c)
}

/// The end of the first token at or after `from`, when one comes before
/// `start`; `from` itself otherwise.
fn after_token(source: &str, from: usize, start: usize) -> usize {
    let text = &source[from..start];
    let mut rest = text;
    loop {
        rest = rest.trim_start();
        if let Some(body) = rest.strip_prefix("/*") {
            match body.find("*/") {
                Some(end) => rest = &body[end + 2..],
                None => return from,
            }
        } else if rest.starts_with("//") {
            match rest.find(is_line_break) {
                Some(end) => rest = &rest[end..],
                None => return from,
            }
        } else {
            return match rest.chars().next() {
                Some(c) => from + (text.len() - rest.len()) + c.len_utf8(),
                None => from,
            };
        }
    }
}

/// Where the token before `start` ends, skipping whitespace and comments
/// backwards. `None` when that crosses a line with `//` on it: a line comment
/// can't be told apart from code when reading backwards.
fn token_end_before(source: &str, start: usize) -> Option<usize> {
    let mut end = start;
    loop {
        let before = &source[..end];
        let trimmed = before.trim_end();
        if before[trimmed.len()..].contains(is_line_break) {
            let line_start = trimmed.rfind(is_line_break).map_or(0, |i| i + 1);
            if trimmed[line_start..].contains("//") {
                return None;
            }
        }
        end = trimmed.len();
        match trimmed.strip_suffix("*/") {
            Some(inner) => end = inner.rfind("/*")?,
            None => return Some(end),
        }
    }
}

/// The comments at the start of `text`, up to the first line break, as
/// TypeScript prints them there: `/* */` followed by a space, `//` by a new
/// line (indented like the `.d.ts` class members). Comments spanning lines,
/// which TypeScript re-indents, are dropped with the rest.
fn same_line_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    loop {
        rest = rest.trim_start_matches(is_space);
        if let Some(body) = rest.strip_prefix("/*") {
            let Some(end) = body.find("*/") else { break };
            let comment = &rest[..end + 4];
            if comment.contains(is_line_break) {
                return String::new();
            }
            out.push_str(comment);
            out.push(' ');
            rest = &rest[comment.len()..];
        } else {
            if rest.starts_with("//") {
                let end = rest.find(is_line_break).unwrap_or(rest.len());
                out.push_str(&rest[..end]);
                out.push_str("\n    ");
            }
            break;
        }
    }
    out
}

/// A string literal as TypeScript prints a synthesized one: double quotes,
/// escapes for control characters and everything outside ASCII (as UTF-16
/// code units).
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"),
            '\u{c}' => out.push_str("\\f"),
            '\u{8}' => out.push_str("\\b"),
            // `\0` followed by a digit would read as an octal escape.
            '\0' if chars.peek().is_some_and(char::is_ascii_digit) => out.push_str("\\x00"),
            '\0' => out.push_str("\\0"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7f => {
                for unit in c.encode_utf16(&mut [0; 2]) {
                    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("\\u{unit:04X}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
