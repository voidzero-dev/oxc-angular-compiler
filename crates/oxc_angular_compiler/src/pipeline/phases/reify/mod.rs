//! Reify phase.
//!
//! Converts IR operations to Output AST statements.
//!
//! After this phase, the create_statements and update_statements fields
//! of each ViewCompilationUnit are populated with the generated JavaScript
//! statements that call the Angular runtime instructions.
//!
//! Ported from Angular's `template/pipeline/src/phases/reify.ts`.

mod angular_expression;
pub(crate) mod ir_expression;
mod statements;
mod utils;

use oxc_allocator::{Box, Vec as OxcVec};
use oxc_diagnostics::OxcDiagnostic;
use oxc_span::Atom;
use rustc_hash::FxHashMap;

use crate::ast::expression::AngularExpression;
use crate::ir::enums::BindingKind;
use crate::ir::expression::IrExpression;
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, FnParam, FunctionExpr, InvokeFunctionExpr, LiteralExpr,
    LiteralValue, OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr, ReturnStatement,
    clone_output_statement,
};
use crate::pipeline::compilation::{
    ComponentCompilationJob, HostBindingCompilationJob, TemplateCompilationMode,
    ViewCompilationUnit,
};
use crate::pipeline::constant_pool::ConstantPool;
use crate::pipeline::expression_store::ExpressionStore;

use angular_expression::convert_angular_expression;
use ir_expression::convert_ir_expression;
use statements::*;
use utils::strip_prefix;

use crate::output::ast::ExpressionStatement;

/// Converts an OutputStatement that may contain WrappedIrNode expressions
/// to a proper OutputStatement with all IR expressions converted.
fn convert_statement_ir_nodes<'a>(
    allocator: &'a oxc_allocator::Allocator,
    stmt: &OutputStatement<'a>,
    expressions: &ExpressionStore<'a>,
    root_xref: XrefId,
    diagnostics: &mut Vec<OxcDiagnostic>,
) -> OutputStatement<'a> {
    match stmt {
        OutputStatement::Return(ret) => {
            let converted_expr =
                convert_output_expr_ir_nodes(allocator, &ret.value, expressions, root_xref);
            OutputStatement::Return(Box::new_in(
                ReturnStatement { value: converted_expr, source_span: ret.source_span },
                allocator,
            ))
        }
        OutputStatement::Expression(expr_stmt) => {
            let converted_expr =
                convert_output_expr_ir_nodes(allocator, &expr_stmt.expr, expressions, root_xref);
            OutputStatement::Expression(Box::new_in(
                ExpressionStatement { expr: converted_expr, source_span: expr_stmt.source_span },
                allocator,
            ))
        }
        // Other statement types pass through (they don't contain WrappedIrNode)
        OutputStatement::DeclareVar(_)
        | OutputStatement::DeclareFunction(_)
        | OutputStatement::If(_) => {
            // These shouldn't appear in listener handler_ops.
            // Emit a warning for unexpected statement types.
            diagnostics.push(OxcDiagnostic::warn(
                "Unexpected statement type in handler_ops. Only Return and Expression statements are expected."
            ));
            // Return the statement unchanged as a fallback
            clone_output_statement(stmt, allocator)
        }
    }
}

/// Converts an OutputExpression that may be a WrappedIrNode to a proper OutputExpression.
fn convert_output_expr_ir_nodes<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: &OutputExpression<'a>,
    expressions: &ExpressionStore<'a>,
    root_xref: XrefId,
) -> OutputExpression<'a> {
    match expr {
        OutputExpression::WrappedIrNode(wrapped) => {
            // Convert the wrapped IR expression to a proper output expression
            convert_ir_expression(allocator, &wrapped.node, expressions, root_xref)
        }
        // All other expressions are already proper output expressions
        // Clone them to get owned values
        other => other.clone_in(allocator),
    }
}

/// Context for reifying operations, holding information needed across views.
struct ReifyContext<'a> {
    /// Map from view xref to its function name (set by naming phase).
    view_fn_names: FxHashMap<XrefId, Atom<'a>>,
    /// Map from view xref to its declaration count.
    view_decls: FxHashMap<XrefId, u32>,
    /// Map from view xref to its variable count.
    view_vars: FxHashMap<XrefId, u32>,
    /// Template compilation mode (Full or DomOnly).
    mode: TemplateCompilationMode,
}

/// Reifies IR expressions to Output AST.
///
/// This phase converts all IR operations into output statements containing
/// runtime instruction calls (ɵɵelementStart, ɵɵtext, ɵɵproperty, etc.).
pub fn reify(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;
    let root_xref = job.root.xref;
    let mode = job.mode;
    let mut diagnostics = Vec::new();

    // Build context with view function names, decl counts, and var counts.
    // This allows us to reference embedded view functions when reifying TemplateOp/RepeaterCreate/Projection.
    let mut view_fn_names = FxHashMap::default();
    let mut view_decls = FxHashMap::default();
    let mut view_vars = FxHashMap::default();
    for view in job.all_views() {
        if let Some(fn_name) = &view.fn_name {
            view_fn_names.insert(view.xref, fn_name.clone());
        }
        if let Some(decl_count) = view.decl_count {
            view_decls.insert(view.xref, decl_count);
        }
        if let Some(vars) = view.vars {
            view_vars.insert(view.xref, vars);
        }
    }
    let ctx = ReifyContext { view_fn_names, view_decls, view_vars, mode };

    // Collect xrefs of embedded views (excluding root) before splitting borrows
    let embedded_xrefs: std::vec::Vec<XrefId> =
        job.all_views().filter(|v| v.xref != root_xref).map(|v| v.xref).collect();

    // Process root view first using split borrow
    {
        let ComponentCompilationJob { expressions, root, pool, .. } = job;
        let (create_stmts, update_stmts) = reify_view_to_stmts(
            allocator,
            root,
            expressions,
            pool,
            root_xref,
            &ctx,
            &mut diagnostics,
        );
        root.create_statements.extend(create_stmts);
        root.update_statements.extend(update_stmts);
    }

    // Process embedded views using split borrow
    for xref in embedded_xrefs {
        let ComponentCompilationJob { expressions, views, pool, .. } = job;
        if let Some(view) = views.get_mut(&xref) {
            let view = view.as_mut();
            let (create_stmts, update_stmts) = reify_view_to_stmts(
                allocator,
                view,
                expressions,
                pool,
                root_xref,
                &ctx,
                &mut diagnostics,
            );
            view.create_statements.extend(create_stmts);
            view.update_statements.extend(update_stmts);
        }
    }

    job.diagnostics.extend(diagnostics);
}

/// Reify a single view's operations to statements (without modifying the view).
fn reify_view_to_stmts<'a>(
    allocator: &'a oxc_allocator::Allocator,
    view: &mut ViewCompilationUnit<'a>,
    expressions: &ExpressionStore<'a>,
    pool: &mut ConstantPool<'a>,
    root_xref: XrefId,
    ctx: &ReifyContext<'a>,
    diagnostics: &mut Vec<OxcDiagnostic>,
) -> (std::vec::Vec<OutputStatement<'a>>, std::vec::Vec<OutputStatement<'a>>) {
    let mut create_stmts = std::vec::Vec::new();
    let mut update_stmts = std::vec::Vec::new();

    // Reify create operations
    // Use iter_mut() so we can take ownership of expressions that can't be cloned
    for op in view.create.iter_mut() {
        let stmt = reify_create_op(allocator, op, expressions, pool, root_xref, ctx, diagnostics);
        if let Some(s) = stmt {
            create_stmts.push(s);
        }
    }

    // Reify update operations
    for op in view.update.iter() {
        let stmt = reify_update_op(allocator, op, expressions, root_xref, ctx.mode, diagnostics);
        if let Some(s) = stmt {
            update_stmts.push(s);
        }
    }

    (create_stmts, update_stmts)
}

/// Reify a single create operation to a statement.
fn reify_create_op<'a>(
    allocator: &'a oxc_allocator::Allocator,
    op: &mut CreateOp<'a>,
    expressions: &ExpressionStore<'a>,
    pool: &mut ConstantPool<'a>,
    root_xref: XrefId,
    ctx: &ReifyContext<'a>,
    diagnostics: &mut Vec<OxcDiagnostic>,
) -> Option<OutputStatement<'a>> {
    let is_dom_only = ctx.mode == TemplateCompilationMode::DomOnly;

    match op {
        CreateOp::ElementStart(elem) => {
            let slot = elem.slot.map(|s| s.0).unwrap_or(0);
            let local_refs_index = elem.local_refs_index;
            if is_dom_only {
                Some(create_dom_element_start_stmt(
                    allocator,
                    &elem.tag,
                    slot,
                    elem.attributes,
                    local_refs_index,
                ))
            } else {
                Some(create_element_start_stmt(
                    allocator,
                    &elem.tag,
                    slot,
                    elem.attributes,
                    local_refs_index,
                ))
            }
        }
        CreateOp::Element(elem) => {
            let slot = elem.slot.map(|s| s.0).unwrap_or(0);
            let local_refs_index = elem.local_refs_index;
            if is_dom_only {
                Some(create_dom_element_stmt(
                    allocator,
                    &elem.tag,
                    slot,
                    elem.attributes,
                    local_refs_index,
                ))
            } else {
                Some(create_element_stmt(
                    allocator,
                    &elem.tag,
                    slot,
                    elem.attributes,
                    local_refs_index,
                ))
            }
        }
        CreateOp::ElementEnd(_) => {
            if is_dom_only {
                Some(create_dom_element_end_stmt(allocator))
            } else {
                Some(create_element_end_stmt(allocator))
            }
        }
        CreateOp::Text(text) => {
            let slot = text.slot.map(|s| s.0).unwrap_or(0);
            let initial_value = if text.initial_value.as_str().is_empty() {
                None
            } else {
                Some(text.initial_value.as_str())
            };
            Some(create_text_stmt(allocator, slot, initial_value))
        }
        CreateOp::Template(tmpl) => {
            // Look up the function name for this template's embedded view
            let fn_name = ctx.view_fn_names.get(&tmpl.embedded_view).cloned();
            let decls = tmpl.decl_count;
            let vars = tmpl.vars;
            let slot = tmpl.slot.map(|s| s.0).unwrap_or(0);
            let tag = tmpl.tag.as_ref();
            let attributes = tmpl.attributes;
            let local_refs_index = tmpl.local_refs_index;
            // Block templates can't have directives so we can always generate them as DOM-only.
            // In DomOnly mode, all templates use domTemplate because there are no directive deps.
            // This matches Angular's reify.ts:
            //   op.templateKind === ir.TemplateKind.Block || unit.job.mode === TemplateCompilationMode.DomOnly
            let use_dom_template =
                tmpl.template_kind == crate::ir::enums::TemplateKind::Block || is_dom_only;
            if use_dom_template {
                Some(create_dom_template_stmt(
                    allocator,
                    slot,
                    fn_name,
                    decls,
                    vars,
                    tag,
                    attributes,
                    local_refs_index,
                ))
            } else {
                Some(create_template_stmt(
                    allocator,
                    slot,
                    fn_name,
                    decls,
                    vars,
                    tag,
                    attributes,
                    local_refs_index,
                ))
            }
        }
        CreateOp::Listener(listener) => {
            // Convert handler_ops to statements for the handler function
            let mut handler_stmts = OxcVec::new_in(allocator);

            // Process handler_ops if present (new approach aligned with Angular)
            // These include Variable ops (e.g., RestoreView) added by save_restore_view phase
            for handler_op in listener.handler_ops.iter() {
                if let UpdateOp::Statement(stmt_op) = handler_op {
                    // Convert WrappedIrNode expressions in the statement to output expressions
                    let converted_stmt = convert_statement_ir_nodes(
                        allocator,
                        &stmt_op.statement,
                        expressions,
                        root_xref,
                        diagnostics,
                    );
                    handler_stmts.push(converted_stmt);
                } else if let Some(stmt) = reify_update_op(
                    allocator,
                    handler_op,
                    expressions,
                    root_xref,
                    ctx.mode,
                    diagnostics,
                ) {
                    handler_stmts.push(stmt);
                }
            }

            // Always add handler_expression as a return statement at the end
            // This is the actual event handler logic
            if let Some(handler_expr) = &listener.handler_expression {
                let output_expr =
                    convert_ir_expression(allocator, handler_expr, expressions, root_xref);
                // Use return statement so the handler returns the result
                handler_stmts.push(OutputStatement::Return(Box::new_in(
                    ReturnStatement { value: output_expr, source_span: None },
                    allocator,
                )));
            }

            // Parse global event target (window, document, body) if present
            let event_target = listener
                .event_target
                .as_ref()
                .and_then(|target| GlobalEventTarget::from_str(target.as_str()));
            // Use domListener in DomOnly mode when not a host listener or animation listener
            let use_dom_listener =
                is_dom_only && !listener.host_listener && !listener.is_animation_listener;
            // use_capture is true when both host_listener and is_animation_listener are true
            let use_capture = listener.host_listener && listener.is_animation_listener;
            if use_dom_listener {
                Some(create_dom_listener_stmt_with_handler(
                    allocator,
                    &listener.name,
                    handler_stmts,
                    event_target,
                    listener.handler_fn_name.as_ref(),
                    listener.consumes_dollar_event,
                ))
            } else {
                Some(create_listener_stmt_with_handler(
                    allocator,
                    &listener.name,
                    handler_stmts,
                    event_target,
                    use_capture,
                    listener.handler_fn_name.as_ref(),
                    listener.consumes_dollar_event,
                ))
            }
        }
        CreateOp::Projection(proj) => {
            let slot = proj.slot.map(|s| s.0).unwrap_or(0);
            let projection_slot_index = proj.projection_slot_index;

            // Get fallback view info if present
            let (fallback_fn_name, fallback_decls, fallback_vars) =
                if let Some(fallback_xref) = proj.fallback {
                    let fn_name = ctx.view_fn_names.get(&fallback_xref).map(|a| a.as_str());
                    let decls = ctx.view_decls.get(&fallback_xref).copied();
                    let vars = ctx.view_vars.get(&fallback_xref).copied();
                    (fn_name, decls, vars)
                } else {
                    (None, None, None)
                };

            // Take ownership of attributes (moves it out of proj)
            let attributes = std::mem::take(&mut proj.attributes);

            Some(create_projection_stmt(
                allocator,
                slot,
                projection_slot_index,
                attributes,
                fallback_fn_name,
                fallback_decls,
                fallback_vars,
            ))
        }
        CreateOp::Container(container) => {
            let slot = container.slot.map(|s| s.0).unwrap_or(0);
            let attributes = container.attributes;
            let local_refs_index = container.local_refs_index;
            if is_dom_only {
                Some(create_dom_container_stmt(allocator, slot, attributes, local_refs_index))
            } else {
                Some(create_container_stmt(allocator, slot, attributes, local_refs_index))
            }
        }
        CreateOp::ContainerEnd(_) => {
            if is_dom_only {
                Some(create_dom_container_end_stmt(allocator))
            } else {
                Some(create_container_end_stmt(allocator))
            }
        }
        CreateOp::DeclareLet(decl) => {
            let slot = decl.slot.map(|s| s.0).unwrap_or(0);
            Some(create_declare_let_stmt(allocator, slot))
        }
        CreateOp::Conditional(cond) => {
            // Emit ɵɵconditionalCreate instruction for the first branch in @if/@switch
            // Look up the function name for this branch's view
            let fn_name = ctx.view_fn_names.get(&cond.xref).cloned();
            let slot = cond.slot.map(|s| s.0).unwrap_or(0);
            Some(create_conditional_create_stmt(
                allocator,
                slot,
                fn_name,
                cond.decls,
                cond.vars,
                cond.tag.as_ref(),
                cond.attributes,
            ))
        }
        CreateOp::RepeaterCreate(repeater) => {
            // Emit repeaterCreate instruction for @for
            // Look up the function name for the repeater's body view (not xref!)
            let fn_name = ctx.view_fn_names.get(&repeater.body_view).cloned();
            // Look up empty view function name if present
            let empty_fn_name =
                repeater.empty_view.and_then(|ev| ctx.view_fn_names.get(&ev).cloned());
            let slot = repeater.slot.map(|s| s.0).unwrap_or(0);

            // Generate track function if not already set by optimization phase
            // Ported from Angular's reifyTrackBy() in reify.ts
            let track_fn_expr = reify_track_by(
                allocator,
                pool,
                expressions,
                root_xref,
                &repeater.track,
                repeater.track_fn_name.as_ref(),
                repeater.uses_component_instance,
                &mut repeater.track_by_ops,
                diagnostics,
            );

            Some(create_repeater_create_stmt_with_track_expr(
                allocator,
                slot,
                fn_name,
                repeater.decls,
                repeater.vars,
                repeater.tag.as_ref(),
                repeater.attributes,
                track_fn_expr,
                repeater.uses_component_instance,
                empty_fn_name,
                repeater.empty_decl_count,
                repeater.empty_var_count,
                repeater.empty_tag.as_ref(),
                repeater.empty_attributes,
            ))
        }
        CreateOp::Pipe(pipe) => {
            // Emit pipe instruction
            let slot = pipe.slot.map(|s| s.0).unwrap_or(0);
            Some(create_pipe_stmt(allocator, slot, &pipe.name))
        }
        CreateOp::Defer(defer) => {
            // Emit defer instruction for @defer
            let slot = defer.slot.map(|s| s.0).unwrap_or(0);
            // Extract const indices from resolved ConstReference expressions.
            // By this point, Phase 53 (collectConstExpressions) has replaced
            // ConstCollectedExpr with ConstReference(index).
            let loading_config = defer.loading_config.as_ref().and_then(|expr| {
                if let crate::ir::expression::IrExpression::ConstReference(cr) = expr.as_ref() {
                    Some(cr.index)
                } else {
                    None
                }
            });
            let placeholder_config = defer.placeholder_config.as_ref().and_then(|expr| {
                if let crate::ir::expression::IrExpression::ConstReference(cr) = expr.as_ref() {
                    Some(cr.index)
                } else {
                    None
                }
            });
            Some(create_defer_stmt(
                allocator,
                slot,
                defer.main_slot.map(|s| s.0),
                defer.resolver_fn.take(),
                defer.loading_slot.map(|s| s.0),
                defer.placeholder_slot.map(|s| s.0),
                defer.error_slot.map(|s| s.0),
                loading_config,
                placeholder_config,
                defer.flags,
            ))
        }
        CreateOp::DeferOn(defer_on) => {
            // Emit deferOn instruction based on trigger kind
            let options = defer_on
                .options
                .as_ref()
                .map(|expr| convert_ir_expression(allocator, expr, expressions, root_xref));
            Some(create_defer_on_stmt(
                allocator,
                defer_on.trigger,
                defer_on.target_slot.map(|s| s.0),
                defer_on.target_slot_view_steps,
                defer_on.modifier,
                defer_on.delay,
                options,
            ))
        }
        CreateOp::I18nStart(i18n) => {
            // Emit i18nStart instruction
            let slot = i18n.slot.map(|s| s.0).unwrap_or(0);
            Some(create_i18n_start_stmt(
                allocator,
                slot,
                i18n.message_index,
                i18n.sub_template_index,
            ))
        }
        CreateOp::I18n(i18n) => {
            // Emit i18n instruction
            let slot = i18n.slot.map(|s| s.0).unwrap_or(0);
            Some(create_i18n_stmt(allocator, slot, i18n.message_index, i18n.sub_template_index))
        }
        CreateOp::I18nEnd(_) => {
            // Emit i18nEnd instruction
            Some(create_i18n_end_stmt(allocator))
        }
        CreateOp::Namespace(ns) => {
            // Emit namespace change instruction
            Some(create_namespace_stmt(allocator, ns.active))
        }
        CreateOp::ProjectionDef(proj_def) => {
            // Emit projectionDef instruction with R3 format def expression
            Some(create_projection_def_stmt_from_expr(allocator, proj_def.def.as_ref()))
        }
        CreateOp::DisableBindings(_) => {
            // Emit disableBindings instruction
            Some(create_disable_bindings_stmt(allocator))
        }
        CreateOp::EnableBindings(_) => {
            // Emit enableBindings instruction
            Some(create_enable_bindings_stmt(allocator))
        }
        CreateOp::TwoWayListener(listener) => {
            // Emit twoWayListener instruction
            // Convert handler_ops to statements for the handler function
            let mut handler_stmts = OxcVec::new_in(allocator);
            for handler_op in listener.handler_ops.iter() {
                // Handle Statement ops specially to convert WrappedIrNode expressions
                if let UpdateOp::Statement(stmt_op) = handler_op {
                    // Convert WrappedIrNode expressions in the statement to output expressions
                    let converted_stmt = convert_statement_ir_nodes(
                        allocator,
                        &stmt_op.statement,
                        expressions,
                        root_xref,
                        diagnostics,
                    );
                    handler_stmts.push(converted_stmt);
                } else if let Some(stmt) = reify_update_op(
                    allocator,
                    handler_op,
                    expressions,
                    root_xref,
                    ctx.mode,
                    diagnostics,
                ) {
                    handler_stmts.push(stmt);
                }
            }
            Some(create_two_way_listener_stmt(
                allocator,
                &listener.name,
                handler_stmts,
                listener.handler_fn_name.as_ref(),
            ))
        }
        CreateOp::AnimationListener(listener) => {
            // Emit syntheticHostListener instruction for animation listeners
            let mut handler_stmts = OxcVec::new_in(allocator);
            for handler_op in listener.handler_ops.iter() {
                if let Some(stmt) = reify_update_op(
                    allocator,
                    handler_op,
                    expressions,
                    root_xref,
                    ctx.mode,
                    diagnostics,
                ) {
                    handler_stmts.push(stmt);
                }
            }
            Some(create_animation_listener_stmt(
                allocator,
                &listener.name,
                listener.phase,
                handler_stmts,
                listener.handler_fn_name.as_ref(),
                listener.consumes_dollar_event,
            ))
        }
        CreateOp::AnimationString(anim) => {
            // Emit ɵɵanimateEnter or ɵɵanimateLeave instruction for animation string bindings
            let expr = convert_ir_expression(allocator, &anim.expression, expressions, root_xref);
            Some(create_animation_string_stmt(allocator, anim.animation_kind, expr))
        }
        CreateOp::Animation(anim) => {
            // Emit ɵɵanimateEnter or ɵɵanimateLeave instruction for animation bindings (Value kind)
            // The handler_ops contain a return statement with the expression
            let mut handler_stmts = OxcVec::new_in(allocator);
            for handler_op in anim.handler_ops.iter() {
                if let Some(stmt) = reify_update_op(
                    allocator,
                    handler_op,
                    expressions,
                    root_xref,
                    ctx.mode,
                    diagnostics,
                ) {
                    handler_stmts.push(stmt);
                }
            }
            Some(create_animation_op_stmt(
                allocator,
                anim.animation_kind,
                handler_stmts,
                anim.handler_fn_name.as_ref(),
            ))
        }
        CreateOp::Variable(var) => {
            // Emit variable declaration with initializer
            // All Variable ops use `const` (StmtModifier::Final), matching Angular's reify.ts
            let value = convert_ir_expression(allocator, &var.initializer, expressions, root_xref);
            Some(create_variable_decl_stmt_with_value(allocator, &var.name, value))
        }
        CreateOp::ContainerStart(container) => {
            // Emit elementContainerStart instruction
            let slot = container.slot.map(|s| s.0).unwrap_or(0);
            let attributes = container.attributes;
            let local_refs_index = container.local_refs_index;
            if is_dom_only {
                Some(create_dom_container_start_stmt(allocator, slot, attributes, local_refs_index))
            } else {
                Some(create_container_start_stmt(allocator, slot, attributes, local_refs_index))
            }
        }
        CreateOp::I18nAttributes(i18n_attrs) => {
            // Emit ɵɵi18nAttributes instruction only if config is set
            if let Some(config_index) = i18n_attrs.i18n_attributes_config {
                use crate::ir::ops::I18nSlotHandle;
                let slot = match i18n_attrs.handle {
                    I18nSlotHandle::Single(slot_id) => slot_id.0,
                    I18nSlotHandle::Range(start, _) => start.0,
                };
                Some(create_i18n_attributes_stmt(allocator, slot, config_index))
            } else {
                None
            }
        }
        CreateOp::ConditionalBranch(branch) => {
            // Emit ɵɵconditionalBranchCreate instruction for branches after the first in @if/@switch
            // Look up the function name for this branch's view
            let fn_name = ctx.view_fn_names.get(&branch.xref).cloned();
            let slot = branch.slot.map(|s| s.0).unwrap_or(0);
            Some(create_conditional_branch_create_stmt(
                allocator,
                slot,
                fn_name,
                branch.decls,
                branch.vars,
                branch.tag.as_ref(),
                branch.attributes,
            ))
        }
        CreateOp::ControlCreate(_) => {
            // Emit ɵɵcontrolCreate instruction for control binding initialization
            Some(create_control_create_stmt(allocator))
        }
        // Non-emitting operations
        CreateOp::ListEnd(_)
        | CreateOp::I18nMessage(_)
        | CreateOp::I18nContext(_)
        | CreateOp::IcuStart(_)
        | CreateOp::IcuEnd(_)
        | CreateOp::IcuPlaceholder(_)
        | CreateOp::ExtractedAttribute(_)
        | CreateOp::SourceLocation(_)
        | CreateOp::Statement(_) => None,
    }
}

/// Reify a single update operation to a statement.
fn reify_update_op<'a>(
    allocator: &'a oxc_allocator::Allocator,
    op: &UpdateOp<'a>,
    expressions: &ExpressionStore<'a>,
    root_xref: XrefId,
    mode: TemplateCompilationMode,
    diagnostics: &mut Vec<OxcDiagnostic>,
) -> Option<OutputStatement<'a>> {
    let is_dom_only = mode == TemplateCompilationMode::DomOnly;

    match op {
        UpdateOp::Property(prop) => {
            // Angular uses property() with nested interpolate*() calls for interpolated properties.
            // The interpolation is handled by convert_ir_expression which generates
            // ɵɵinterpolate*() calls when the expression is an Interpolation.
            // Example: [title]="Hello {{name}}" -> ɵɵproperty("title", ɵɵinterpolate1("Hello ", name, ""))
            let expr = convert_ir_expression(allocator, &prop.expression, expressions, root_xref);
            // In DomOnly mode, use domProperty unless it's an animation binding
            // Matches Angular's reify.ts line 613-621
            let is_animation =
                matches!(prop.binding_kind, BindingKind::LegacyAnimation | BindingKind::Animation);
            if is_dom_only && !is_animation {
                Some(create_dom_property_stmt(allocator, &prop.name, expr, prop.sanitizer.as_ref()))
            } else if is_aria_attribute(prop.name.as_str()) {
                // Use ɵɵariaProperty for ARIA attributes (e.g., aria-label, aria-hidden)
                Some(create_aria_property_stmt(allocator, &prop.name, expr))
            } else {
                Some(create_property_stmt_with_expr(
                    allocator,
                    &prop.name,
                    expr,
                    prop.sanitizer.as_ref(),
                ))
            }
        }
        UpdateOp::InterpolateText(interp) => {
            // Handle multiple interpolations like "{{a}} and {{b}}"
            let (args, expr_count) =
                reify_interpolation(allocator, &interp.interpolation, expressions, root_xref);
            Some(create_text_interpolate_stmt_with_args(allocator, args, expr_count))
        }
        UpdateOp::Binding(binding) => {
            let expr =
                convert_ir_expression(allocator, &binding.expression, expressions, root_xref);
            Some(create_binding_stmt_with_expr(allocator, &binding.name, expr))
        }
        UpdateOp::StyleProp(style) => {
            let expr = convert_ir_expression(allocator, &style.expression, expressions, root_xref);
            // Strip "style." prefix if present
            let name = strip_prefix(&style.name, "style.");
            Some(create_style_prop_stmt_with_expr(allocator, &name, expr, style.unit.as_ref()))
        }
        UpdateOp::ClassProp(class) => {
            let expr = convert_ir_expression(allocator, &class.expression, expressions, root_xref);
            // Strip "class." prefix if present
            let name = strip_prefix(&class.name, "class.");
            Some(create_class_prop_stmt_with_expr(allocator, &name, expr))
        }
        UpdateOp::Attribute(attr) => {
            let expr = convert_ir_expression(allocator, &attr.expression, expressions, root_xref);
            // Strip "attr." prefix if present
            let name = strip_prefix(&attr.name, "attr.");
            Some(create_attribute_stmt_with_expr(
                allocator,
                &name,
                expr,
                attr.sanitizer.as_ref(),
                attr.namespace.as_ref(),
            ))
        }
        UpdateOp::Advance(adv) => Some(create_advance_stmt(allocator, adv.delta)),
        UpdateOp::StoreLet(store) => {
            // StoreLet as an update op should have been converted to a StoreLet expression
            // during the store_let_optimization phase. If it reaches reify, it's a compiler bug.
            // This matches Angular's behavior which throws: "AssertionError: unexpected storeLet"
            diagnostics.push(OxcDiagnostic::error(format!(
                "AssertionError: unexpected storeLet {}",
                store.declared_name
            )));
            None
        }
        UpdateOp::TwoWayProperty(twp) => {
            let expr = convert_ir_expression(allocator, &twp.expression, expressions, root_xref);
            Some(create_two_way_property_stmt(allocator, &twp.name, expr, twp.sanitizer.as_ref()))
        }
        UpdateOp::Repeater(rep) => {
            let expr = convert_ir_expression(allocator, &rep.collection, expressions, root_xref);
            Some(create_repeater_stmt(allocator, expr))
        }
        UpdateOp::Conditional(cond) => {
            // Use processed expression (built by conditionals phase).
            // Angular asserts that processed is always set by this point
            // (throws "Conditional test was not set." in reify.ts:698).
            let expr = if let Some(ref processed) = cond.processed {
                convert_ir_expression(allocator, processed, expressions, root_xref)
            } else {
                diagnostics
                    .push(OxcDiagnostic::error("AssertionError: Conditional test was not set."));
                return None;
            };
            let context_value = cond
                .context_value
                .as_ref()
                .map(|cv| convert_ir_expression(allocator, cv, expressions, root_xref));
            Some(create_conditional_update_stmt(allocator, expr, context_value))
        }
        UpdateOp::StyleMap(style) => {
            let expr = convert_ir_expression(allocator, &style.expression, expressions, root_xref);
            Some(create_style_map_stmt(allocator, expr))
        }
        UpdateOp::ClassMap(class) => {
            let expr = convert_ir_expression(allocator, &class.expression, expressions, root_xref);
            Some(create_class_map_stmt(allocator, expr))
        }
        UpdateOp::DomProperty(prop) => {
            let expr = convert_ir_expression(allocator, &prop.expression, expressions, root_xref);
            // Animation bindings use syntheticHostProperty instead of domProperty
            // Matches Angular's reify.ts lines 662-675
            let is_animation =
                matches!(prop.binding_kind, BindingKind::LegacyAnimation | BindingKind::Animation);
            if is_animation {
                Some(create_animation_stmt(allocator, &prop.name, expr))
            } else {
                Some(create_dom_property_stmt(allocator, &prop.name, expr, prop.sanitizer.as_ref()))
            }
        }
        UpdateOp::I18nExpression(i18n) => {
            let expr = convert_ir_expression(allocator, &i18n.expression, expressions, root_xref);
            Some(create_i18n_exp_stmt(allocator, expr))
        }
        UpdateOp::I18nApply(i18n) => {
            // Use the slot from handle, matching Angular's reify.ts line 648:
            // `ng.i18nApply(op.handle.slot!, op.sourceSpan)`
            let slot = match i18n.handle {
                crate::ir::ops::I18nSlotHandle::Single(slot_id) => slot_id.0,
                crate::ir::ops::I18nSlotHandle::Range(start, _) => start.0,
            };
            Some(create_i18n_apply_stmt(allocator, slot))
        }
        UpdateOp::AnimationBinding(anim) => {
            let expr = convert_ir_expression(allocator, &anim.expression, expressions, root_xref);
            Some(create_animation_binding_stmt(allocator, &anim.name, expr))
        }
        UpdateOp::Control(ctrl) => {
            let expr = convert_ir_expression(allocator, &ctrl.expression, expressions, root_xref);
            Some(create_control_stmt(allocator, expr))
        }
        UpdateOp::Variable(var) => {
            // Emit variable declaration with initializer for update phase
            // All Variable ops use `const` (StmtModifier::Final), matching Angular's reify.ts
            let value = convert_ir_expression(allocator, &var.initializer, expressions, root_xref);
            Some(create_variable_decl_stmt_with_value(allocator, &var.name, value))
        }
        UpdateOp::DeferWhen(defer_when) => {
            // Emit deferWhen runtime instruction with condition
            let expr =
                convert_ir_expression(allocator, &defer_when.condition, expressions, root_xref);
            Some(create_defer_when_stmt(allocator, defer_when.modifier, expr))
        }
        // Non-emitting or already handled operations
        UpdateOp::ListEnd(_) => None,
        // Statement ops contain pre-built OutputStatements (e.g., temp variable declarations,
        // or side-effectful expressions from variable optimization).
        // These may contain WrappedIrNode expressions that need to be converted.
        UpdateOp::Statement(stmt_op) => Some(convert_statement_ir_nodes(
            allocator,
            &stmt_op.statement,
            expressions,
            root_xref,
            diagnostics,
        )),
    }
}

/// Reify an interpolation expression to arguments for textInterpolate.
fn reify_interpolation<'a>(
    allocator: &'a oxc_allocator::Allocator,
    interpolation: &IrExpression<'a>,
    expressions: &ExpressionStore<'a>,
    root_xref: XrefId,
) -> (OxcVec<'a, OutputExpression<'a>>, usize) {
    match interpolation {
        IrExpression::Interpolation(ir_interp) => {
            // Direct IR interpolation - interleave strings and expressions
            let expr_count = ir_interp.expressions.len();
            let mut args = OxcVec::new_in(allocator);

            // For single expression with empty surrounding strings, use simple form
            if expr_count == 1 && ir_interp.strings.iter().all(|s| s.is_empty()) {
                args.push(convert_ir_expression(
                    allocator,
                    &ir_interp.expressions[0],
                    expressions,
                    root_xref,
                ));
            } else {
                for (i, expr) in ir_interp.expressions.iter().enumerate() {
                    if i < ir_interp.strings.len() {
                        args.push(OutputExpression::Literal(Box::new_in(
                            LiteralExpr {
                                value: LiteralValue::String(ir_interp.strings[i].clone()),
                                source_span: None,
                            },
                            allocator,
                        )));
                    }
                    args.push(convert_ir_expression(allocator, expr, expressions, root_xref));
                }
                if ir_interp.strings.len() > ir_interp.expressions.len() {
                    if let Some(trailing) = ir_interp.strings.last() {
                        // Only add trailing string if it's not empty
                        // (Angular drops trailing empty strings and the runtime handles it)
                        if !trailing.is_empty() {
                            args.push(OutputExpression::Literal(Box::new_in(
                                LiteralExpr {
                                    value: LiteralValue::String(trailing.clone()),
                                    source_span: None,
                                },
                                allocator,
                            )));
                        }
                    }
                }
            }
            (args, expr_count)
        }
        IrExpression::ExpressionRef(id) => {
            // Look up the stored AngularExpression
            let angular_expr = expressions.get(*id);
            if let AngularExpression::Interpolation(ang_interp) = angular_expr {
                // Angular interpolation - interleave strings and expressions
                let expr_count = ang_interp.expressions.len();
                let mut args = OxcVec::new_in(allocator);

                // For single expression with empty surrounding strings, use simple form
                if expr_count == 1 && ang_interp.strings.iter().all(|s| s.is_empty()) {
                    args.push(convert_angular_expression(
                        allocator,
                        &ang_interp.expressions[0],
                        root_xref,
                    ));
                } else {
                    for (i, expr) in ang_interp.expressions.iter().enumerate() {
                        if i < ang_interp.strings.len() {
                            args.push(OutputExpression::Literal(Box::new_in(
                                LiteralExpr {
                                    value: LiteralValue::String(ang_interp.strings[i].clone()),
                                    source_span: None,
                                },
                                allocator,
                            )));
                        }
                        args.push(convert_angular_expression(allocator, expr, root_xref));
                    }
                    if ang_interp.strings.len() > ang_interp.expressions.len() {
                        if let Some(trailing) = ang_interp.strings.last() {
                            // Only add trailing string if it's not empty
                            // (Angular drops trailing empty strings and the runtime handles it)
                            if !trailing.is_empty() {
                                args.push(OutputExpression::Literal(Box::new_in(
                                    LiteralExpr {
                                        value: LiteralValue::String(trailing.clone()),
                                        source_span: None,
                                    },
                                    allocator,
                                )));
                            }
                        }
                    }
                }
                (args, expr_count)
            } else {
                // Not an interpolation - convert as single expression
                let mut args = OxcVec::new_in(allocator);
                args.push(convert_angular_expression(allocator, angular_expr, root_xref));
                (args, 1)
            }
        }
        _ => {
            // Other expression types - convert directly
            let mut args = OxcVec::new_in(allocator);
            args.push(convert_ir_expression(allocator, interpolation, expressions, root_xref));
            (args, 1)
        }
    }
}

/// Reifies IR expressions to Output AST for host binding compilation.
///
/// This is the host binding version that works with HostBindingCompilationJob.
/// Host bindings are simpler than templates - they don't have element creation,
/// just property/attribute bindings and event listeners.
pub fn reify_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;
    let root_xref = job.root.xref;
    let mut diagnostics = Vec::new();

    // Reify create operations (listeners)
    for op in job.root.create.iter() {
        let stmt =
            reify_host_create_op(allocator, op, &job.expressions, root_xref, &mut diagnostics);
        if let Some(s) = stmt {
            job.root.create_statements.push(s);
        }
    }

    // Reify update operations (property bindings)
    // Host bindings use Full mode (not DomOnly)
    for op in job.root.update.iter() {
        let stmt = reify_update_op(
            allocator,
            op,
            &job.expressions,
            root_xref,
            TemplateCompilationMode::Full,
            &mut diagnostics,
        );
        if let Some(s) = stmt {
            job.root.update_statements.push(s);
        }
    }

    job.diagnostics.extend(diagnostics);
}

/// Reify a single create operation for host bindings.
fn reify_host_create_op<'a>(
    allocator: &'a oxc_allocator::Allocator,
    op: &CreateOp<'a>,
    expressions: &ExpressionStore<'a>,
    root_xref: XrefId,
    diagnostics: &mut Vec<OxcDiagnostic>,
) -> Option<OutputStatement<'a>> {
    match op {
        CreateOp::Listener(listener) => {
            // Convert handler_ops to statements for the handler function
            let mut handler_stmts = OxcVec::new_in(allocator);
            for handler_op in listener.handler_ops.iter() {
                // Host bindings use Full mode (not DomOnly)
                if let UpdateOp::Statement(stmt_op) = handler_op {
                    // Convert WrappedIrNode expressions in the statement to output expressions
                    let converted_stmt = convert_statement_ir_nodes(
                        allocator,
                        &stmt_op.statement,
                        expressions,
                        root_xref,
                        diagnostics,
                    );
                    handler_stmts.push(converted_stmt);
                } else if let Some(stmt) = reify_update_op(
                    allocator,
                    handler_op,
                    expressions,
                    root_xref,
                    TemplateCompilationMode::Full,
                    diagnostics,
                ) {
                    handler_stmts.push(stmt);
                }
            }

            // Add handler_expression as a return statement at the end
            // This is the actual event handler logic
            if let Some(handler_expr) = &listener.handler_expression {
                let output_expr =
                    convert_ir_expression(allocator, handler_expr, expressions, root_xref);
                // Use return statement so the handler returns the result
                handler_stmts.push(OutputStatement::Return(Box::new_in(
                    ReturnStatement { value: output_expr, source_span: None },
                    allocator,
                )));
            }

            // Parse global event target (window, document, body) if present
            let event_target = listener
                .event_target
                .as_ref()
                .and_then(|target| GlobalEventTarget::from_str(target.as_str()));

            // Host listeners use ɵɵlistener, NOT ɵɵsyntheticHostListener
            // syntheticHostListener is only for animation listeners
            // Use capture mode only when both host_listener and is_animation_listener are true
            let use_capture = listener.host_listener && listener.is_animation_listener;
            Some(create_listener_stmt_with_handler(
                allocator,
                &listener.name,
                handler_stmts,
                event_target,
                use_capture,
                listener.handler_fn_name.as_ref(),
                listener.consumes_dollar_event,
            ))
        }
        CreateOp::AnimationListener(listener) => {
            // Emit syntheticHostListener for animation listeners
            let mut handler_stmts = OxcVec::new_in(allocator);
            for handler_op in listener.handler_ops.iter() {
                // Host bindings use Full mode (not DomOnly)
                if let Some(stmt) = reify_update_op(
                    allocator,
                    handler_op,
                    expressions,
                    root_xref,
                    TemplateCompilationMode::Full,
                    diagnostics,
                ) {
                    handler_stmts.push(stmt);
                }
            }
            Some(create_animation_listener_stmt(
                allocator,
                &listener.name,
                listener.phase,
                handler_stmts,
                listener.handler_fn_name.as_ref(),
                listener.consumes_dollar_event,
            ))
        }
        // ExtractedAttribute ops are handled separately for hostAttrs
        CreateOp::ExtractedAttribute(_) => None,
        // Other create ops are not relevant for host bindings
        _ => None,
    }
}

/// Reifies the tracking expression of a RepeaterCreateOp.
///
/// If the track function was already set by the optimization phase (e.g., for
/// `track $index` -> `ɵɵrepeaterTrackByIndex`), returns a reference to that.
/// Otherwise, generates a track function with params ($index, $item) and
/// the track expression as the body, registers it with the constant pool,
/// and returns the function reference.
///
/// When `track_by_ops` is `Some`, the ops are reified into statements and assembled
/// into a function body. This handles cases where the track expression needs additional
/// context variable declarations (e.g., `nextContext()` calls for outer-scope access).
///
/// Ported from Angular's `reifyTrackBy()` in `reify.ts`.
#[allow(clippy::too_many_arguments)]
fn reify_track_by<'a>(
    allocator: &'a oxc_allocator::Allocator,
    pool: &mut ConstantPool<'a>,
    expressions: &ExpressionStore<'a>,
    root_xref: XrefId,
    track: &IrExpression<'a>,
    track_fn_name: Option<&Atom<'a>>,
    uses_component_instance: bool,
    track_by_ops: &mut Option<oxc_allocator::Vec<'a, UpdateOp<'a>>>,
    diagnostics: &mut Vec<OxcDiagnostic>,
) -> OutputExpression<'a> {
    // If the tracking function was already set by optimization phase, return a reference to it
    if let Some(fn_name) = track_fn_name {
        // Angular runtime functions (starting with ɵɵ) need the i0. namespace prefix
        if fn_name.starts_with("ɵɵ") {
            return OutputExpression::ReadProp(Box::new_in(
                ReadPropExpr {
                    receiver: Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: Atom::from("i0"), source_span: None },
                            allocator,
                        )),
                        allocator,
                    ),
                    name: fn_name.clone(),
                    optional: false,
                    source_span: None,
                },
                allocator,
            ));
        }
        // Handle property access like "ctx.trackByFn" or "componentInstance().fn"
        if let Some(dot_pos) = fn_name.find('.') {
            // Allocate the parts into the arena first
            let receiver_name_str = allocator.alloc_str(&fn_name[..dot_pos]);
            let prop_name_str = allocator.alloc_str(&fn_name[dot_pos + 1..]);
            let receiver_name = Atom::from(receiver_name_str);
            let prop_name = Atom::from(prop_name_str);

            // Check if receiver ends with "()" indicating a function call
            if receiver_name.ends_with("()") {
                // componentInstance().fn pattern
                let call_name =
                    Atom::from(allocator.alloc_str(&receiver_name[..receiver_name.len() - 2]));
                return OutputExpression::ReadProp(Box::new_in(
                    ReadPropExpr {
                        receiver: Box::new_in(
                            OutputExpression::InvokeFunction(Box::new_in(
                                InvokeFunctionExpr {
                                    fn_expr: Box::new_in(
                                        OutputExpression::ReadProp(Box::new_in(
                                            ReadPropExpr {
                                                receiver: Box::new_in(
                                                    OutputExpression::ReadVar(Box::new_in(
                                                        ReadVarExpr {
                                                            name: Atom::from("i0"),
                                                            source_span: None,
                                                        },
                                                        allocator,
                                                    )),
                                                    allocator,
                                                ),
                                                name: call_name,
                                                optional: false,
                                                source_span: None,
                                            },
                                            allocator,
                                        )),
                                        allocator,
                                    ),
                                    args: OxcVec::new_in(allocator),
                                    pure: false,
                                    optional: false,
                                    source_span: None,
                                },
                                allocator,
                            )),
                            allocator,
                        ),
                        name: prop_name,
                        optional: false,
                        source_span: None,
                    },
                    allocator,
                ));
            }
            // Simple property access like "ctx.trackByFn"
            return OutputExpression::ReadProp(Box::new_in(
                ReadPropExpr {
                    receiver: Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: receiver_name, source_span: None },
                            allocator,
                        )),
                        allocator,
                    ),
                    name: prop_name,
                    optional: false,
                    source_span: None,
                },
                allocator,
            ));
        }
        return OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: fn_name.clone(), source_span: None },
            allocator,
        ));
    }

    // Create the track function with params ($index, $item)
    let mut params = OxcVec::with_capacity_in(2, allocator);
    params.push(FnParam { name: Atom::from("$index") });
    params.push(FnParam { name: Atom::from("$item") });

    let fn_expr = if let Some(track_ops) = track_by_ops {
        // Complex case: track_by_ops is present (set by track_fn_optimization phase).
        // This happens when the track expression needs additional ops like context
        // variable declarations (e.g., `const group_r2 = nextContext().$implicit`).
        //
        // Ported from Angular's reify.ts lines 884-904:
        //   reifyUpdateOperations(unit, op.trackByOps);
        //   const statements = [...]; // from trackByOps
        //   fn = op.usesComponentInstance || statements.length !== 1 || !(statements[0] instanceof ReturnStatement)
        //     ? o.fn(params, statements)
        //     : o.arrowFn(params, statements[0].value);

        // Reify each op in track_by_ops into output statements
        let mut statements = OxcVec::new_in(allocator);
        for track_op in track_ops.iter() {
            if let Some(stmt) = reify_update_op(
                allocator,
                track_op,
                expressions,
                root_xref,
                TemplateCompilationMode::Full,
                diagnostics,
            ) {
                statements.push(stmt);
            }
        }

        // Determine whether to use function or arrow:
        // Angular uses function when:
        //   - usesComponentInstance is true, OR
        //   - there are multiple statements, OR
        //   - the single statement is not a ReturnStatement
        let use_function = uses_component_instance
            || statements.len() != 1
            || !matches!(statements.first(), Some(OutputStatement::Return(_)));

        if use_function {
            OutputExpression::Function(Box::new_in(
                FunctionExpr { name: None, params, statements, source_span: None },
                allocator,
            ))
        } else {
            // Single return statement → extract value for arrow function body
            // Clone the return value since we can't move out of the Box
            let return_value = if let Some(OutputStatement::Return(ret)) = statements.first() {
                ret.value.clone_in(allocator)
            } else {
                unreachable!("checked above that there's exactly one Return statement");
            };

            OutputExpression::ArrowFunction(Box::new_in(
                ArrowFunctionExpr {
                    params,
                    body: ArrowFunctionBody::Expression(Box::new_in(return_value, allocator)),
                    source_span: None,
                },
                allocator,
            ))
        }
    } else {
        // Simple case: no track_by_ops. Wrap the raw track expression.
        // Ported from Angular's reify.ts lines 878-883:
        //   fn = op.usesComponentInstance
        //     ? o.fn(params, [new o.ReturnStatement(op.track)])
        //     : o.arrowFn(params, op.track);
        let track_body = convert_ir_expression(allocator, track, expressions, root_xref);

        if uses_component_instance {
            let mut stmts = OxcVec::with_capacity_in(1, allocator);
            stmts.push(OutputStatement::Return(Box::new_in(
                ReturnStatement { value: track_body, source_span: None },
                allocator,
            )));

            OutputExpression::Function(Box::new_in(
                FunctionExpr { name: None, params, statements: stmts, source_span: None },
                allocator,
            ))
        } else {
            OutputExpression::ArrowFunction(Box::new_in(
                ArrowFunctionExpr {
                    params,
                    body: ArrowFunctionBody::Expression(Box::new_in(track_body, allocator)),
                    source_span: None,
                },
                allocator,
            ))
        }
    };

    // Register with constant pool as a shared function
    // This matches Angular TypeScript: op.trackByFn = unit.job.pool.getSharedFunctionReference(fn, '_forTrack')
    pool.get_shared_function_reference(fn_expr, "_forTrack", true)
}
