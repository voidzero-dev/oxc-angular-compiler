//! Prints a TypeScript type the way ngtsc writes it into a `.d.ts`: through
//! its `TypeEmitter` (`@angular/core` names become `i0.Name`, string literals
//! are re-quoted) and TypeScript's printer (normalised spacing).
//!
//! Used for `static ngAcceptInputType_x: T;`, where `T` is the type of the
//! input transform's first parameter.

use oxc_ast::ast::{
    TSLiteral, TSSignature, TSTupleElement, TSType, TSTypeName, TSTypeOperatorOperator,
    TSTypeQueryExprName,
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
    pub(crate) fn print(&mut self, ty: &TSType<'_>) -> String {
        match ty {
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
                let mut out = self.type_name(&r.type_name);
                if let Some(args) = &r.type_arguments {
                    out.push_str(&self.type_args(&args.params));
                }
                out
            }
            TSType::TSUnionType(u) => self.join(&u.types, " | "),
            TSType::TSIntersectionType(i) => self.join(&i.types, " & "),
            TSType::TSParenthesizedType(p) => format!("({})", self.print(&p.type_annotation)),
            TSType::TSArrayType(a) => format!("{}[]", self.print(&a.element_type)),
            TSType::TSIndexedAccessType(i) => {
                format!("{}[{}]", self.print(&i.object_type), self.print(&i.index_type))
            }
            TSType::TSTypeOperatorType(o) => {
                let op = match o.operator {
                    TSTypeOperatorOperator::Keyof => "keyof",
                    TSTypeOperatorOperator::Unique => "unique",
                    TSTypeOperatorOperator::Readonly => "readonly",
                };
                format!("{op} {}", self.print(&o.type_annotation))
            }
            TSType::TSTupleType(t) => {
                let items: Vec<String> =
                    t.element_types.iter().map(|e| self.tuple_element(e)).collect();
                format!("[{}]", items.join(", "))
            }
            TSType::TSNamedTupleMember(m) => self.named_tuple_member(m),
            TSType::TSLiteralType(l) => self.literal(&l.literal),
            TSType::TSTemplateLiteralType(t) => {
                let mut out = String::from("`");
                for (i, quasi) in t.quasis.iter().enumerate() {
                    out.push_str(quasi.value.raw.as_str());
                    if let Some(ty) = t.types.get(i) {
                        out.push_str("${");
                        out.push_str(&self.print(ty));
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
                    TSTypeQueryExprName::QualifiedName(q) => {
                        format!("typeof {}", self.slice(q.span()))
                    }
                    other => format!("typeof {}", self.slice(other.span())),
                };
                if let Some(args) = &q.type_arguments {
                    out.push_str(&self.type_args(&args.params));
                }
                out
            }
            TSType::TSTypeLiteral(l) if l.members.is_empty() => "{}".into(),
            TSType::TSTypeLiteral(l) => {
                let members: Vec<String> = l.members.iter().map(|m| self.signature(m)).collect();
                format!("{{ {} }}", members.join(" "))
            }
            TSType::TSFunctionType(f) => {
                let type_params =
                    f.type_parameters.as_ref().map_or(String::new(), |t| self.slice(t.span));
                format!(
                    "{type_params}({}) => {}",
                    self.params(&f.params, f.this_param.as_ref().map(|t| t.span)),
                    self.print(&f.return_type.type_annotation)
                )
            }
            TSType::TSConstructorType(c) => {
                let type_params =
                    c.type_parameters.as_ref().map_or(String::new(), |t| self.slice(t.span));
                format!(
                    "{}new {type_params}({}) => {}",
                    if c.r#abstract { "abstract " } else { "" },
                    self.params(&c.params, None),
                    self.print(&c.return_type.type_annotation)
                )
            }
            TSType::TSConditionalType(c) => format!(
                "{} extends {} ? {} : {}",
                self.print(&c.check_type),
                self.print(&c.extends_type),
                self.print(&c.true_type),
                self.print(&c.false_type)
            ),
            // Mapped, infer, predicate, import and JSDoc types are printed as written.
            other => self.slice(other.span()),
        }
    }

    fn join(&mut self, types: &[TSType<'_>], separator: &str) -> String {
        types.iter().map(|t| self.print(t)).collect::<Vec<_>>().join(separator)
    }

    fn type_args(&mut self, params: &[TSType<'_>]) -> String {
        format!("<{}>", self.join(params, ", "))
    }

    /// `@angular/core` names become `i0.Name`; local and global names stay as
    /// written; names from other modules set `other_module`.
    fn type_name(&mut self, name: &TSTypeName<'_>) -> String {
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
                    other => return self.slice(other.span()) + &rest,
                }
            }
            TSTypeName::ThisExpression(_) => return "this".into(),
        };
        match self.scope.import(head) {
            Some(import) if import.module == "@angular/core" => match import.imported {
                Some(imported) => format!("i0.{imported}{rest}"),
                None => format!("i0{rest}"),
            },
            Some(_) => {
                self.other_module = true;
                format!("{head}{rest}")
            }
            None => format!("{head}{rest}"),
        }
    }

    fn tuple_element(&mut self, element: &TSTupleElement<'_>) -> String {
        match element {
            TSTupleElement::TSOptionalType(o) => format!("{}?", self.print(&o.type_annotation)),
            TSTupleElement::TSRestType(r) => format!("...{}", self.print(&r.type_annotation)),
            other => self.print(other.to_ts_type()),
        }
    }

    fn named_tuple_member(&mut self, m: &oxc_ast::ast::TSNamedTupleMember<'_>) -> String {
        format!(
            "{}{}: {}",
            m.label.name,
            if m.optional { "?" } else { "" },
            self.tuple_element(&m.element_type)
        )
    }

    fn literal(&self, literal: &TSLiteral<'_>) -> String {
        match literal {
            TSLiteral::BooleanLiteral(b) => b.value.to_string(),
            TSLiteral::NumericLiteral(n) => format_number_like_js(n.value),
            TSLiteral::BigIntLiteral(b) => self.slice(b.span),
            TSLiteral::StringLiteral(s) => format!("\"{}\"", escape_string(&s.value)),
            TSLiteral::TemplateLiteral(t) => self.slice(t.span),
            TSLiteral::UnaryExpression(u) => self.slice(u.span).split_whitespace().collect(),
        }
    }

    fn signature(&mut self, member: &TSSignature<'_>) -> String {
        match member {
            TSSignature::TSPropertySignature(p) => {
                let key = if p.computed {
                    format!("[{}]", self.slice(p.key.span()))
                } else {
                    self.slice(p.key.span())
                };
                let ty = p
                    .type_annotation
                    .as_ref()
                    .map_or(String::new(), |t| format!(": {}", self.print(&t.type_annotation)));
                format!(
                    "{}{key}{}{ty};",
                    if p.readonly { "readonly " } else { "" },
                    if p.optional { "?" } else { "" },
                )
            }
            other => {
                let text = self.slice(other.span());
                let text = text.trim_end_matches([';', ',']).to_string();
                format!("{};", text.split_whitespace().collect::<Vec<_>>().join(" "))
            }
        }
    }

    fn params(
        &mut self,
        params: &oxc_ast::ast::FormalParameters<'_>,
        this: Option<oxc_span::Span>,
    ) -> String {
        let mut out: Vec<String> = this.map(|s| self.slice(s)).into_iter().collect();
        for param in &params.items {
            let mut text = self.slice(param.pattern.span());
            if param.optional {
                text.push('?');
            }
            if let Some(t) = &param.type_annotation {
                text.push_str(": ");
                text.push_str(&self.print(&t.type_annotation));
            }
            out.push(text);
        }
        if let Some(rest) = &params.rest {
            let mut text = format!("...{}", self.slice(rest.rest.argument.span()));
            if let Some(t) = &rest.type_annotation {
                text.push_str(": ");
                text.push_str(&self.print(&t.type_annotation));
            }
            out.push(text);
        }
        out.join(", ")
    }

    fn slice(&self, span: oxc_span::Span) -> String {
        span.source_text(self.source).to_string()
    }
}

fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}
