//! Resolve i18n attribute sanitizers phase.
//!
//! Wraps static i18n extracted attributes in their corresponding
//! sanitizers/validators, e.g. `<a href="..." i18n-href>` becomes
//! `["href", i0.ɵɵsanitizeUrl(i18n_0)]` in the consts array.
//!
//! Ported from Angular's `template/pipeline/src/phases/resolve_i18n_attr_sanitizers.ts`.

use oxc_allocator::{Allocator, Box, Vec as OxcVec};
use oxc_str::Ident;
use rustc_hash::FxHashMap;

use crate::ast::r3::SecurityContext;
use crate::ir::expression::IrExpression;
use crate::ir::ops::{CreateOp, XrefId};
use crate::output::ast::{
    InvokeFunctionExpr, LiteralExpr, LiteralValue, OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::pipeline::compilation::ComponentCompilationJob;
use crate::pipeline::conversion::prefix_with_namespace;
use crate::r3::Identifiers;
use crate::schema::sanitizes_i18n_attrs;

/// Wraps static i18n extracted attributes in their corresponding sanitizers/validators.
///
/// Only Angular versions with compile-time i18n attribute validation run this
/// (see [`sanitizes_i18n_attrs`]).
pub fn resolve_i18n_attr_sanitizers(job: &mut ComponentCompilationJob<'_>) {
    if !sanitizes_i18n_attrs(job.angular_version) {
        return;
    }

    let allocator = job.allocator;
    let core_namespace = job.core_namespace;

    // Upstream keys the `:svg:` / `:math:` qualified tag of every element and
    // template by xref, for `ɵɵvalidateAttribute`'s tag argument.
    let mut tag_names_by_element: FxHashMap<XrefId, String> = FxHashMap::default();
    for view in job.all_views() {
        for op in view.create.iter() {
            let (xref, tag, namespace) = match op {
                CreateOp::ElementStart(op) => (op.xref, Some(&op.tag), op.namespace),
                CreateOp::Template(op) => (op.xref, op.tag.as_ref(), op.namespace),
                _ => continue,
            };
            let tag = tag.map_or("", |tag| tag.as_str());
            tag_names_by_element.insert(xref, prefix_with_namespace(tag, namespace));
        }
    }

    for view in job.all_views_mut() {
        for op in view.create.iter_mut() {
            let CreateOp::ExtractedAttribute(attr) = op else { continue };
            if attr.i18n_context.is_none() || attr.value.is_none() {
                continue;
            }

            let sanitizer = match attr.security_context {
                SecurityContext::Html => Identifiers::SANITIZE_HTML,
                SecurityContext::Style => Identifiers::SANITIZE_STYLE,
                SecurityContext::Script => Identifiers::SANITIZE_SCRIPT,
                SecurityContext::Url => Identifiers::SANITIZE_URL,
                SecurityContext::ResourceUrl => Identifiers::SANITIZE_RESOURCE_URL,
                SecurityContext::AttributeNoBinding => Identifiers::VALIDATE_ATTRIBUTE,
                SecurityContext::None | SecurityContext::UrlOrResourceUrl => continue,
            };
            // The value is the i18n variable read set by `collect_i18n_consts`.
            let Some(value) = attr.value.take() else { continue };
            let expr = match Box::unbox(value) {
                IrExpression::OutputExpr(expr) => Box::unbox(expr),
                other => {
                    attr.value = Some(Box::new_in(other, &allocator));
                    continue;
                }
            };
            let mut args = OxcVec::with_capacity_in(3, &allocator);
            args.push(expr);
            if attr.security_context == SecurityContext::AttributeNoBinding {
                let tag = tag_names_by_element.get(&attr.target).map_or("", String::as_str);
                args.push(string_literal(allocator, Ident::from(allocator.alloc_str(tag))));
                args.push(string_literal(allocator, attr.name));
            }
            let call = call_core_fn(allocator, core_namespace, sanitizer, args);
            attr.value = Some(Box::new_in(
                IrExpression::OutputExpr(Box::new_in(call, &allocator)),
                &allocator,
            ));
        }
    }
}

fn string_literal<'a>(allocator: &'a Allocator, value: Ident<'a>) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(value), source_span: None },
        &allocator,
    ))
}

/// `o.importExpr(Identifiers.<name>).callFn(args)`: `i0.<name>(...args)`.
fn call_core_fn<'a>(
    allocator: &'a Allocator,
    core_namespace: Ident<'a>,
    name: &'static str,
    args: OxcVec<'a, OutputExpression<'a>>,
) -> OutputExpression<'a> {
    let fn_expr = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: core_namespace, source_span: None },
                    &allocator,
                )),
                &allocator,
            ),
            name: Ident::from(name),
            optional: false,
            source_span: None,
        },
        &allocator,
    ));
    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, &allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        &allocator,
    ))
}
