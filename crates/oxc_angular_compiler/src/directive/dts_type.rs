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
    BigIntLiteral, BindingPattern, Expression, FormalParameters, PropertyKey, StringLiteral,
    TSLiteral, TSMappedTypeModifierOperator, TSMethodSignatureKind, TSSignature, TSThisParameter,
    TSTupleElement, TSType, TSTypeAnnotation, TSTypeName, TSTypeOperatorOperator, TSTypeParameter,
    TSTypeParameterDeclaration, TSTypeParameterInstantiation, TSTypePredicateName,
    TSTypeQueryExprName, UnaryOperator,
};
use oxc_span::GetSpan;

use super::evaluator::{AliasTarget, FileScope};
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

impl<'a> TypePrinter<'_, 'a> {
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
                    TSTypeQueryExprName::ThisExpression(_) => "typeof this".into(),
                    TSTypeQueryExprName::TSImportType(_) => return None,
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
            Resolved::Core(rest) => format!("i0{rest}"),
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
                AliasTarget::Module("@angular/core") => {
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
            TSLiteral::StringLiteral(s) => quote_literal(s),
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

    /// A property name, re-quoted like any other string literal. A computed
    /// name (`[token]`, `[ns.token]`) follows the rules of a type name: an
    /// `@angular/core` value becomes `i0.token`, another module's makes the
    /// type `unknown`, and a local or global one stays as written (ngtsc copies
    /// the expression, which doesn't resolve for an import).
    ///
    /// Through an import-equals alias too, although ngtsc copies those as
    /// written: an alias of `@angular/core` gives `i0.token` and one of another
    /// module `unknown`, where ngtsc's name doesn't resolve in the `.d.ts`. An
    /// alias of a global is written as its target (`[Symbol.iterator]` for
    /// `import S = Symbol`), and one of a name the file declares stays as
    /// written, like ngtsc.
    fn key(&mut self, key: &PropertyKey<'_>, computed: bool) -> Option<String> {
        let text = match key {
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
        };
        Some(if computed { format!("[{text}]") } else { text })
    }

    /// A computed property name `a.b.c` (`parts`, head first), as [`Self::key`]
    /// writes it.
    fn computed_name(&mut self, parts: std::vec::Vec<&str>) -> Option<String> {
        let written = parts.join(".");
        Some(match self.resolve_aliases(parts)? {
            Resolved::Core(rest) => format!("i0{rest}"),
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

/// A type name's parts, head first (`["A", "B", "T"]` for `A.B.T`). `None`
/// for one starting with `this`.
fn entity_parts<'n>(name: &'n TSTypeName<'_>, out: &mut Vec<&'n str>) -> Option<()> {
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
            // After the line break, which can be more than one byte (U+2028).
            let line_start = trimmed
                .char_indices()
                .rfind(|&(_, c)| is_line_break(c))
                .map_or(0, |(i, c)| i + c.len_utf8());
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
