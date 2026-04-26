//! I18n const collection phase.
//!
//! Lifts i18n properties into the consts array with dual-mode support
//! for both Closure Compiler (goog.getMsg) and $localize.
//!
//! Ported from Angular's `template/pipeline/src/phases/i18n_const_collection.ts`.

use std::ptr::NonNull;

use oxc_allocator::Vec as ArenaVec;
use oxc_str::Ident;
use rustc_hash::FxHashMap;

use crate::i18n::serializer::format_i18n_placeholder_name;
use crate::ir::enums::I18nExpressionFor;
use crate::ir::i18n_params::I18nParamValue;
use crate::ir::ops::{CreateOp, UpdateOp, XrefId};
use crate::output::ast::{
    LiteralValue, LocalizedStringExpr, OutputExpression, OutputStatement, ReadVarExpr,
};
use crate::pipeline::compilation::{ComponentCompilationJob, ConstValue};
use crate::pipeline::phases::i18n_closure::{
    I18nMessageMeta, create_translation_declaration, generate_closure_var_name,
    generate_file_based_i18n_suffix, generate_i18n_var_name,
};
use crate::r3::Identifiers;

/// The escape sequence used for message param values.
const ESCAPE: char = '\u{FFFD}';

/// Prefix of ICU expressions for post processing.
const I18N_ICU_MAPPING_PREFIX: &str = "I18N_EXP_";

/// Collects i18n constants to the consts array with dual-mode support.
///
/// This phase:
/// 1. Serializes extracted i18n messages into the const array
/// 2. Generates dual-mode code (Closure + $localize) for each message
/// 3. Propagates const indices to i18n ops
pub fn collect_i18n_consts(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Get file-based suffix for Closure variable names
    let file_based_i18n_suffix = job
        .relative_context_file_path
        .as_ref()
        .map(|p| generate_file_based_i18n_suffix(p.as_str()))
        .unwrap_or_else(|| "APP_".to_string());

    // Build up lookup maps

    // Context Xref -> Extracted Attribute target Xrefs
    let mut extracted_attributes_by_i18n_context: FxHashMap<XrefId, Vec<XrefId>> =
        FxHashMap::default();
    // Element/ElementStart Xref -> I18n Attributes op xref
    let mut i18n_attributes_by_element: FxHashMap<XrefId, XrefId> = FxHashMap::default();
    // Element/ElementStart Xref -> All I18n Expression ops for attrs on that target
    let mut i18n_expressions_by_element: FxHashMap<XrefId, Vec<I18nExpressionInfo>> =
        FxHashMap::default();
    // I18n Message Xref -> I18n Message Op info
    let mut messages: FxHashMap<XrefId, MessageInfo> = FxHashMap::default();
    // I18n Context Xref -> Params
    let mut params_by_context: FxHashMap<XrefId, Vec<(String, String)>> = FxHashMap::default();

    // Collect info from all views
    for view in job.all_views() {
        for op in view.create.iter() {
            match op {
                CreateOp::ExtractedAttribute(attr_op) => {
                    if let Some(i18n_context) = attr_op.i18n_context {
                        let attrs =
                            extracted_attributes_by_i18n_context.entry(i18n_context).or_default();
                        attrs.push(attr_op.target);
                    }
                }
                CreateOp::I18nAttributes(i18n_attrs) => {
                    i18n_attributes_by_element.insert(i18n_attrs.target, i18n_attrs.xref);
                }
                CreateOp::I18nMessage(msg_op) => {
                    messages.insert(
                        msg_op.xref,
                        MessageInfo {
                            i18n_context: msg_op.i18n_context,
                            i18n_block: msg_op.i18n_block,
                            message_placeholder: msg_op
                                .message_placeholder
                                .as_ref()
                                .map(|a| a.to_string()),
                            description: msg_op.description.as_ref().map(|a| a.to_string()),
                            meaning: msg_op.meaning.as_ref().map(|a| a.to_string()),
                            custom_id: msg_op.custom_id.as_ref().map(|a| a.to_string()),
                            message_id: msg_op.message_id.as_ref().map(|a| a.to_string()),
                            message_string: msg_op.message_string.as_ref().map(|a| a.to_string()),
                            needs_postprocessing: msg_op.needs_postprocessing,
                            sub_messages: msg_op.sub_messages.iter().copied().collect(),
                        },
                    );
                }
                CreateOp::I18nContext(ctx_op) => {
                    // Collect formatted params from context
                    let formatted = format_context_params(&ctx_op.params);
                    params_by_context.insert(ctx_op.xref, formatted);
                }
                _ => {}
            }
        }

        for op in view.update.iter() {
            if let UpdateOp::I18nExpression(i18n_expr) = op {
                if i18n_expr.usage == I18nExpressionFor::I18nAttribute {
                    let exprs = i18n_expressions_by_element.entry(i18n_expr.target).or_default();
                    let name_str = i18n_expr.name.as_str();
                    if !name_str.is_empty() {
                        exprs.push(I18nExpressionInfo {
                            name: name_str.to_string(),
                            context: i18n_expr.context,
                        });
                    }
                }
            }
        }
    }

    // Step Two: Serialize the extracted i18n messages for root i18n blocks and i18n attributes
    // into the const array using dual-mode code generation.
    //
    // CRITICAL: We must iterate through views and ops in creation order (not HashMap keys)
    // to match Angular's deterministic const index assignment. Angular iterates:
    //   for (const unit of job.units) { for (const op of unit.create) { ... } }
    // See: i18n_const_collection.ts lines 121-151

    // Map: i18n_block xref -> const index
    let mut message_const_indices: FxHashMap<XrefId, u32> = FxHashMap::default();
    // Map: i18n_context xref -> i18n variable name (for attribute bindings)
    let mut i18n_var_names_by_context: FxHashMap<XrefId, Ident<'_>> = FxHashMap::default();

    // Counter for unique variable names
    let mut i18n_var_counter: usize = 0;

    // Collect view xrefs first to avoid borrowing issues
    let view_xrefs_for_messages: Vec<XrefId> = job.all_views().map(|v| v.xref).collect();

    // First collect all the info we need from views (immutable borrow)
    struct MessageToProcess {
        msg_info: MessageInfo,
    }
    let mut messages_to_process: Vec<MessageToProcess> = Vec::new();

    for view_xref in &view_xrefs_for_messages {
        if let Some(view) = job.view(*view_xref) {
            for op in view.create.iter() {
                if let CreateOp::I18nMessage(msg_op) = op {
                    // Skip sub-messages (they'll be handled by their parent)
                    if msg_op.message_placeholder.is_some() {
                        continue;
                    }

                    // Get the pre-collected message info
                    if let Some(msg_info) = messages.get(&msg_op.xref) {
                        messages_to_process.push(MessageToProcess { msg_info: msg_info.clone() });
                    }
                }
            }
        }
    }

    // Now process collected messages and mutate job (mutable borrow)
    for msg_to_process in messages_to_process {
        let msg_info = msg_to_process.msg_info;

        // Collect messages recursively and generate statements
        let (main_var_name, statements) = collect_message(
            allocator,
            &messages,
            &params_by_context,
            &msg_info,
            &file_based_i18n_suffix,
            job.i18n_use_external_ids,
            &mut i18n_var_counter,
        );

        if let Some(block_xref) = msg_info.i18n_block {
            // This is a regular i18n message with a corresponding i18n block.
            // Add to consts array with statements as initializers and record the index.
            let var_name_str = allocator.alloc_str(&main_var_name);
            let main_var = OutputExpression::ReadVar(oxc_allocator::Box::new_in(
                ReadVarExpr { name: Ident::from(var_name_str), source_span: None },
                allocator,
            ));
            let const_index =
                job.add_const_with_initializers(ConstValue::Expression(main_var), statements);
            message_const_indices.insert(block_xref, const_index);
        } else if let Some(ctx_xref) = msg_info.i18n_context {
            // This is an i18n attribute message.
            // Add statements to consts_initializers and save the variable name.
            job.consts_initializers.extend(statements);
            let var_name_str = allocator.alloc_str(&main_var_name);
            let var_name_atom = Ident::from(var_name_str);
            i18n_var_names_by_context.insert(ctx_xref, var_name_atom.clone());

            // This i18n message may correspond to an individual extracted attribute. If so,
            // the value of that attribute is updated to read the extracted i18n variable.
            // This is critical for const_collection to NOT deduplicate attribute arrays that
            // contain different i18n variables.
            // Ported from Angular's i18n_const_collection.ts lines 140-144.
            if extracted_attributes_by_i18n_context.contains_key(&ctx_xref) {
                // Iterate through all views to find and update matching ExtractedAttribute ops
                let view_xrefs_for_attr_update: Vec<XrefId> =
                    job.all_views().map(|v| v.xref).collect();

                for view_xref in view_xrefs_for_attr_update {
                    if let Some(view) = job.view_mut(view_xref) {
                        for op in view.create.iter_mut() {
                            if let CreateOp::ExtractedAttribute(attr_op) = op {
                                if attr_op.i18n_context == Some(ctx_xref) {
                                    // Update the value to reference the i18n variable
                                    let var_expr =
                                        OutputExpression::ReadVar(oxc_allocator::Box::new_in(
                                            ReadVarExpr {
                                                name: var_name_atom.clone(),
                                                source_span: None,
                                            },
                                            allocator,
                                        ));
                                    attr_op.value = Some(oxc_allocator::Box::new_in(
                                        crate::ir::expression::IrExpression::OutputExpr(
                                            oxc_allocator::Box::new_in(var_expr, allocator),
                                        ),
                                        allocator,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Step Three: Serialize I18nAttributes configurations into the const array. Each I18nAttributes
    // instruction has a config array, which contains k-v pairs describing each binding name, and the
    // i18n variable that provides the value.
    //
    // CRITICAL: We must iterate through views and ops in creation order (not HashMap keys)
    // to match Angular's deterministic const index assignment. Angular iterates:
    //   for (const unit of job.units) { for (const elem of unit.create) { ... } }
    // See: i18n_const_collection.ts lines 157-196

    let view_xrefs: Vec<XrefId> =
        std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();

    // Build a map of element xref -> I18nAttributes config to set
    let mut i18n_attrs_configs: FxHashMap<XrefId, u32> = FxHashMap::default();

    // First pass: collect element xrefs and their expressions in view iteration order (immutable borrow)
    // This mirrors Angular's: for (const unit of job.units) { for (const elem of unit.create) { if (ir.isElementOrContainerOp(elem)) { ... } } }
    struct I18nAttrConfigToProcess {
        elem_xref: XrefId,
        unique_expressions: Vec<I18nExpressionInfo>,
    }

    let view_xrefs_for_step3: Vec<XrefId> = job.all_views().map(|v| v.xref).collect();
    let mut configs_to_process: Vec<I18nAttrConfigToProcess> = Vec::new();

    for view_xref in &view_xrefs_for_step3 {
        if let Some(view) = job.view(*view_xref) {
            for op in view.create.iter() {
                // Check if this is an element-or-container op (matches Angular's isElementOrContainerOp)
                let elem_xref = match op {
                    CreateOp::Element(e) => e.xref,
                    CreateOp::ElementStart(e) => e.xref,
                    CreateOp::Container(c) => c.xref,
                    CreateOp::ContainerStart(c) => c.xref,
                    CreateOp::Template(t) => t.xref,
                    CreateOp::RepeaterCreate(r) => r.xref,
                    CreateOp::Conditional(c) => c.xref,
                    CreateOp::ConditionalBranch(c) => c.xref,
                    _ => continue,
                };

                // Check if this element has an I18nAttributes op associated with it
                if i18n_attributes_by_element.get(&elem_xref).is_none() {
                    continue;
                }

                let i18n_expressions = match i18n_expressions_by_element.get(&elem_xref) {
                    Some(exprs) if !exprs.is_empty() => exprs.clone(),
                    _ => continue,
                };

                // Filter for unique property names
                let mut seen_names = rustc_hash::FxHashSet::default();
                let unique_expressions: Vec<_> = i18n_expressions
                    .into_iter()
                    .filter(|expr| {
                        let seen = seen_names.contains(&expr.name);
                        seen_names.insert(expr.name.clone());
                        !seen
                    })
                    .collect();

                if !unique_expressions.is_empty() {
                    configs_to_process
                        .push(I18nAttrConfigToProcess { elem_xref, unique_expressions });
                }
            }
        }
    }

    // Second pass: process collected configs and mutate job (mutable borrow)
    for config in configs_to_process {
        // Build the config array: [name, value, name, value, ...]
        let i18n_attribute_config: Vec<OutputExpression<'_>> = config
            .unique_expressions
            .iter()
            .filter_map(|expr| {
                let i18n_var_name = i18n_var_names_by_context.get(&expr.context)?;

                // Add attribute name as literal
                let name_str = allocator.alloc_str(&expr.name);
                let name_literal = OutputExpression::Literal(oxc_allocator::Box::new_in(
                    crate::output::ast::LiteralExpr {
                        value: LiteralValue::String(Ident::from(name_str)),
                        source_span: None,
                    },
                    allocator,
                ));

                // Add i18n variable reference
                let i18n_var = OutputExpression::ReadVar(oxc_allocator::Box::new_in(
                    ReadVarExpr { name: i18n_var_name.clone(), source_span: None },
                    allocator,
                ));

                Some(vec![name_literal, i18n_var])
            })
            .flatten()
            .collect();

        if !i18n_attribute_config.is_empty() {
            // Create array expression and add to consts
            let mut config_elements = ArenaVec::new_in(allocator);
            config_elements.extend(i18n_attribute_config);

            let config_array = OutputExpression::LiteralArray(oxc_allocator::Box::new_in(
                crate::output::ast::LiteralArrayExpr {
                    entries: config_elements,
                    source_span: None,
                },
                allocator,
            ));
            let const_index = job.add_const(ConstValue::Expression(config_array));
            i18n_attrs_configs.insert(config.elem_xref, const_index);
        }
    }

    // Step Four: Propagate const indices to i18n ops
    for view_xref in &view_xrefs {
        let view = if view_xref.0 == 0 { Some(&mut job.root) } else { job.view_mut(*view_xref) };

        if let Some(view) = view {
            for op in view.create.iter_mut() {
                match op {
                    CreateOp::I18nStart(i18n_op) => {
                        let block_xref = i18n_op.root.unwrap_or(i18n_op.xref);
                        if let Some(&const_idx) = message_const_indices.get(&block_xref) {
                            i18n_op.message_index = Some(const_idx);
                        }
                    }
                    CreateOp::I18n(i18n_op) => {
                        let block_xref = i18n_op.root.unwrap_or(i18n_op.xref);
                        if let Some(&const_idx) = message_const_indices.get(&block_xref) {
                            i18n_op.message_index = Some(const_idx);
                        }
                    }
                    CreateOp::I18nAttributes(i18n_attrs) => {
                        if let Some(&config_idx) = i18n_attrs_configs.get(&i18n_attrs.target) {
                            i18n_attrs.i18n_attributes_config = Some(config_idx);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Step Five: Remove I18nMessage ops (they've been serialized to consts)
    for view_xref in view_xrefs {
        let mut ops_to_remove: Vec<NonNull<CreateOp<'_>>> = Vec::new();

        {
            let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(view_xref) };

            if let Some(view) = view {
                for op in view.create.iter() {
                    if matches!(op, CreateOp::I18nMessage(_)) {
                        ops_to_remove.push(NonNull::from(op));
                    }
                }
            }
        }

        for op_ptr in ops_to_remove {
            if view_xref.0 == 0 {
                unsafe {
                    job.root.create.remove(op_ptr);
                }
            } else if let Some(view) = job.view_mut(view_xref) {
                unsafe {
                    view.create.remove(op_ptr);
                }
            }
        }
    }

    // Note: Extracted attribute expressions are handled directly by the IR.
    // The collected data could be used for optimization in the future.
    drop(extracted_attributes_by_i18n_context);
}

/// Information about an I18nMessage op.
#[derive(Clone)]
struct MessageInfo {
    i18n_context: Option<XrefId>,
    i18n_block: Option<XrefId>,
    message_placeholder: Option<String>,
    description: Option<String>,
    meaning: Option<String>,
    custom_id: Option<String>,
    message_id: Option<String>,
    message_string: Option<String>,
    needs_postprocessing: bool,
    sub_messages: Vec<XrefId>,
}

/// Information about an I18n expression for attributes.
#[derive(Clone)]
struct I18nExpressionInfo {
    name: String,
    context: XrefId,
}

/// Collects a message and its sub-messages, returning the main variable name and statements.
fn collect_message<'a>(
    allocator: &'a oxc_allocator::Allocator,
    messages: &FxHashMap<XrefId, MessageInfo>,
    params_by_context: &FxHashMap<XrefId, Vec<(String, String)>>,
    msg_info: &MessageInfo,
    file_suffix: &str,
    use_external_ids: bool,
    counter: &mut usize,
) -> (String, ArenaVec<'a, OutputStatement<'a>>) {
    let mut all_statements = ArenaVec::new_in(allocator);

    // Recursively collect sub-messages first
    let mut sub_message_placeholders: FxHashMap<String, Vec<String>> = FxHashMap::default();
    for &sub_msg_xref in &msg_info.sub_messages {
        if let Some(sub_msg) = messages.get(&sub_msg_xref) {
            let (sub_var_name, sub_statements) = collect_message(
                allocator,
                messages,
                params_by_context,
                sub_msg,
                file_suffix,
                use_external_ids,
                counter,
            );
            all_statements.extend(sub_statements);

            if let Some(ref placeholder) = sub_msg.message_placeholder {
                sub_message_placeholders.entry(placeholder.clone()).or_default().push(sub_var_name);
            }
        }
    }

    // Get params from context
    let base_params = msg_info
        .i18n_context
        .and_then(|ctx| params_by_context.get(&ctx))
        .cloned()
        .unwrap_or_default();

    // Build postprocessing params from sub-message placeholders (for ICU post-processing)
    let postprocessing_params: Vec<(String, Vec<String>)> = sub_message_placeholders
        .iter()
        .filter(|(_, vars)| vars.len() > 1)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    // Build params with sub-message values
    let mut params = base_params;
    add_sub_message_params(&mut params, &sub_message_placeholders, msg_info.needs_postprocessing);

    // Sort params for consistency
    params.sort_by(|a, b| a.0.cmp(&b.0));

    // Generate variable names
    let i18n_var_name = generate_i18n_var_name(*counter);
    let closure_var_name = generate_closure_var_name(
        msg_info.message_id.as_deref().or(msg_info.custom_id.as_deref()),
        file_suffix,
        *counter,
        use_external_ids,
    );
    *counter += 1;

    // Create metadata for JSDoc
    let meta = if msg_info.description.is_some() || msg_info.meaning.is_some() {
        let desc = msg_info.description.as_ref().map(|d| {
            let s = allocator.alloc_str(d);
            Ident::from(s)
        });
        let meaning = msg_info.meaning.as_ref().map(|m| {
            let s = allocator.alloc_str(m);
            Ident::from(s)
        });
        Some(I18nMessageMeta::new(desc, meaning))
    } else {
        None
    };

    // Serialize message for goog.getMsg format
    // Use stored message_string if available, otherwise fallback to generating from params
    let message_for_closure =
        msg_info.message_string.clone().unwrap_or_else(|| generate_message_from_params(&params));

    // Create $localize expression
    let localized_expr = create_localize_expression(
        allocator,
        &message_for_closure,
        &params,
        msg_info.description.clone(),
        msg_info.meaning.clone(),
        msg_info.custom_id.clone(),
    );

    // Wrap with postprocess if needed
    let localized_expr = if msg_info.needs_postprocessing || !postprocessing_params.is_empty() {
        wrap_with_postprocess(allocator, localized_expr, &postprocessing_params)
    } else {
        localized_expr
    };

    // Generate dual-mode translation declaration
    let i18n_var_atom = Ident::from(allocator.alloc_str(&i18n_var_name));
    let closure_var_atom = Ident::from(allocator.alloc_str(&closure_var_name));

    let statements = create_translation_declaration(
        allocator,
        i18n_var_atom,
        closure_var_atom,
        &message_for_closure,
        &params,
        localized_expr,
        meta.as_ref(),
    );

    all_statements.extend(statements);

    (i18n_var_name, all_statements)
}

/// Add sub-message placeholder values to the params.
fn add_sub_message_params(
    params: &mut Vec<(String, String)>,
    sub_message_placeholders: &FxHashMap<String, Vec<String>>,
    _needs_postprocessing: bool,
) {
    for (placeholder, sub_vars) in sub_message_placeholders {
        if sub_vars.len() == 1 {
            // Single sub-message: use its variable directly
            // The value will be the variable reference (handled at runtime)
            params.push((placeholder.clone(), format!("{ESCAPE}{}{ESCAPE}", sub_vars[0])));
        } else {
            // Multiple sub-messages: create ICU mapping placeholder for post-processing
            params.push((
                placeholder.clone(),
                format!("{ESCAPE}{I18N_ICU_MAPPING_PREFIX}{placeholder}{ESCAPE}"),
            ));
        }
    }
}

/// Generate a message string from params (fallback when message AST is not available).
fn generate_message_from_params(params: &[(String, String)]) -> String {
    let mut result = String::new();
    for (name, _value) in params {
        let formatted_name = format_i18n_placeholder_name(name, true);
        result.push_str(&format!("{{${formatted_name}}}"));
    }
    result
}

/// Format params from an I18nContext into (placeholder, value) pairs.
fn format_context_params(
    params: &oxc_allocator::HashMap<'_, Ident<'_>, ArenaVec<'_, I18nParamValue>>,
) -> Vec<(String, String)> {
    use crate::pipeline::phases::extract_i18n_messages::format_params;

    let params_vec: Vec<(Ident<'_>, Vec<I18nParamValue>)> =
        params.iter().map(|(k, v)| (k.clone(), v.iter().copied().collect())).collect();

    let (formatted, _needs_postprocessing) = format_params(&params_vec);
    formatted
}

/// Create a $localize tagged template literal expression.
///
/// The message_string contains the message text with placeholders like `{$INTERPOLATION}`.
/// We parse this to extract the text parts between placeholders.
///
/// For a simple message like "Hello World" with custom_id "my-id":
/// - cooked[0] = ":@@my-id:Hello World"
///
/// For a message with interpolation like "Hello, {$INTERPOLATION}!":
/// - cooked[0] = ":@@my-id:Hello, "
/// - cooked[1] = ":INTERPOLATION:!"
/// - expressions[0] = the interpolation value
fn create_localize_expression<'a>(
    allocator: &'a oxc_allocator::Allocator,
    message_string: &str,
    params: &[(String, String)],
    description: Option<String>,
    meaning: Option<String>,
    custom_id: Option<String>,
) -> OutputExpression<'a> {
    // Parse message_string to extract text parts and placeholder names in order
    let (text_parts, placeholder_order) = parse_message_string(message_string);

    let mut message_parts = ArenaVec::new_in(allocator);
    let mut placeholder_names = ArenaVec::new_in(allocator);
    let mut expressions = ArenaVec::new_in(allocator);

    // Build a map from placeholder name to value for quick lookup
    let params_map: FxHashMap<String, String> =
        params.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

    // First message part: includes metadata block + first text segment
    // Format: ":meaning|description@@customId:text"
    let first_text = text_parts.first().map(|s| s.as_str()).unwrap_or("");
    let head_cooked = serialize_i18n_head(first_text, &meaning, &description, &custom_id);
    let head_str = allocator.alloc_str(&head_cooked);
    message_parts.push(Ident::from(head_str));

    // Subsequent parts: ":PLACEHOLDER_NAME:text"
    for (i, placeholder) in placeholder_order.iter().enumerate() {
        // Format placeholder name (UPPERCASE for $localize)
        let formatted_name = format_i18n_placeholder_name(placeholder, false);
        let name_str = allocator.alloc_str(&formatted_name);
        placeholder_names.push(Ident::from(name_str));

        // Get the value for this placeholder
        // The params_map is keyed by the original placeholder name, but the message_string
        // uses camelCase (from format_i18n_placeholder_name with use_camel_case=true).
        // We need to find the matching param key.
        let value = find_param_value(&params_map, placeholder);
        let value_str = allocator.alloc_str(&value);
        let literal_expr = OutputExpression::Literal(oxc_allocator::Box::new_in(
            crate::output::ast::LiteralExpr {
                value: LiteralValue::String(Ident::from(value_str)),
                source_span: None,
            },
            allocator,
        ));
        expressions.push(literal_expr);

        // Text part after this placeholder
        let text_part = text_parts.get(i + 1).map(|s| s.as_str()).unwrap_or("");
        let part_cooked = serialize_i18n_template_part(&formatted_name, text_part);
        let part_str = allocator.alloc_str(&part_cooked);
        message_parts.push(Ident::from(part_str));
    }

    // Store metadata for potential future use (JSDoc generation in emitter)
    let desc_atom = description.map(|d| {
        let s = allocator.alloc_str(&d);
        Ident::from(s)
    });
    let meaning_atom = meaning.map(|m| {
        let s = allocator.alloc_str(&m);
        Ident::from(s)
    });
    let custom_id_atom = custom_id.map(|c| {
        let s = allocator.alloc_str(&c);
        Ident::from(s)
    });

    OutputExpression::LocalizedString(oxc_allocator::Box::new_in(
        LocalizedStringExpr {
            description: desc_atom,
            meaning: meaning_atom,
            custom_id: custom_id_atom,
            message_parts,
            placeholder_names,
            expressions,
            source_span: None,
        },
        allocator,
    ))
}

/// Parse a message string to extract text parts and placeholder names.
///
/// Message format: "text{$PLACEHOLDER}more text{$ANOTHER}end"
/// Returns: (["text", "more text", "end"], ["PLACEHOLDER", "ANOTHER"])
fn parse_message_string(message: &str) -> (Vec<String>, Vec<String>) {
    let mut text_parts = Vec::new();
    let mut placeholders = Vec::new();
    let mut current_text = String::new();
    let mut chars = message.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' && chars.peek() == Some(&'$') {
            // Start of placeholder: {$NAME}
            text_parts.push(current_text);
            current_text = String::new();

            // Skip the '$'
            chars.next();

            // Collect placeholder name until '}'
            let mut name = String::new();
            for c in chars.by_ref() {
                if c == '}' {
                    break;
                }
                name.push(c);
            }
            placeholders.push(name);
        } else {
            current_text.push(ch);
        }
    }

    // Don't forget the last text part
    text_parts.push(current_text);

    (text_parts, placeholders)
}

/// Serialize the i18n head (first message part) with metadata.
///
/// Format: ":meaning|description@@customId:text"
/// - meaning and description are separated by |
/// - customId is prefixed with @@
/// - If there's no metadata, just return the text (with starting colon escaped if needed)
fn serialize_i18n_head(
    text: &str,
    meaning: &Option<String>,
    description: &Option<String>,
    custom_id: &Option<String>,
) -> String {
    let mut meta_block = String::new();

    // Build meta block: meaning|description@@customId
    if let Some(m) = meaning {
        meta_block.push_str(m);
    }
    if meaning.is_some() || description.is_some() {
        if meaning.is_some() {
            meta_block.push('|');
        }
        if let Some(d) = description {
            meta_block.push_str(d);
        }
    }
    if let Some(id) = custom_id {
        meta_block.push_str("@@");
        meta_block.push_str(id);
    }

    if meta_block.is_empty() {
        // No metadata - just return text (escape starting colon if needed)
        if text.starts_with(':') { format!("\\:{}", &text[1..]) } else { text.to_string() }
    } else {
        // With metadata: :meta:text
        format!(":{}:{}", meta_block, text)
    }
}

/// Serialize an i18n template part (after first part).
///
/// Format: ":PLACEHOLDER_NAME:text"
fn serialize_i18n_template_part(placeholder_name: &str, text: &str) -> String {
    format!(":{}:{}", placeholder_name, text)
}

/// Find the parameter value for a placeholder name from the message string.
///
/// The message_string uses camelCase placeholder names (e.g., `interpolation`),
/// but the params_map is keyed by the original placeholder names (e.g., `INTERPOLATION`).
/// This function tries to find the matching param key by comparing the formatted names.
fn find_param_value(params_map: &FxHashMap<String, String>, placeholder_name: &str) -> String {
    // First try direct lookup
    if let Some(value) = params_map.get(placeholder_name) {
        return value.clone();
    }

    // Try UPPERCASE lookup first since that's the most common format
    let uppercase_name = format_i18n_placeholder_name(placeholder_name, false);
    if let Some(value) = params_map.get(&uppercase_name) {
        return value.clone();
    }

    // Try to find a key that matches when formatted to camelCase
    for (key, value) in params_map {
        let formatted_key = format_i18n_placeholder_name(key, true);
        if formatted_key == placeholder_name {
            return value.clone();
        }
    }

    // Fallback to empty string if no match found
    String::new()
}

/// Wrap an i18n expression with i18nPostprocess for ICU message handling.
fn wrap_with_postprocess<'a>(
    allocator: &'a oxc_allocator::Allocator,
    expr: OutputExpression<'a>,
    postprocessing_params: &[(String, Vec<String>)],
) -> OutputExpression<'a> {
    use crate::output::ast::{
        InvokeFunctionExpr, LiteralArrayExpr, LiteralMapEntry, LiteralMapExpr,
    };

    // Create ɵɵi18nPostprocess function reference (i0.ɵɵi18nPostprocess)
    let fn_var = OutputExpression::ReadProp(oxc_allocator::Box::new_in(
        crate::output::ast::ReadPropExpr {
            receiver: oxc_allocator::Box::new_in(
                OutputExpression::ReadVar(oxc_allocator::Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(Identifiers::I18N_POSTPROCESS),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // Create args array with the localized expression
    let mut args = ArenaVec::new_in(allocator);
    args.push(expr);

    // Add postprocessing params if any
    if !postprocessing_params.is_empty() {
        let mut entries = ArenaVec::new_in(allocator);
        for (placeholder, var_names) in postprocessing_params {
            // Format placeholder name
            let formatted_name = format_i18n_placeholder_name(placeholder, false);
            let key_str = allocator.alloc_str(&formatted_name);

            // Create array of variable references
            let mut var_refs = ArenaVec::new_in(allocator);
            for var_name in var_names {
                let var_str = allocator.alloc_str(var_name);
                var_refs.push(OutputExpression::ReadVar(oxc_allocator::Box::new_in(
                    ReadVarExpr { name: Ident::from(var_str), source_span: None },
                    allocator,
                )));
            }

            entries.push(LiteralMapEntry {
                key: Ident::from(key_str),
                value: OutputExpression::LiteralArray(oxc_allocator::Box::new_in(
                    LiteralArrayExpr { entries: var_refs, source_span: None },
                    allocator,
                )),
                quoted: true,
            });
        }

        args.push(OutputExpression::LiteralMap(oxc_allocator::Box::new_in(
            LiteralMapExpr { entries, source_span: None },
            allocator,
        )));
    }

    // Create the function call: ɵɵi18nPostprocess(localizedExpr, params?)
    OutputExpression::InvokeFunction(oxc_allocator::Box::new_in(
        InvokeFunctionExpr {
            fn_expr: oxc_allocator::Box::new_in(fn_var, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::ast::{LiteralExpr, OutputExpression, ReadVarExpr};
    use crate::output::emitter::JsEmitter;
    use oxc_allocator::Allocator;
    use oxc_str::Ident;

    #[test]
    fn test_wrap_with_postprocess_uses_namespace_prefix() {
        // Regression test for bug where wrap_with_postprocess() created a bare
        // ReadVar(ɵɵi18nPostprocess) instead of ReadProp(i0.ɵɵi18nPostprocess).
        // At runtime this caused: ReferenceError: ɵɵi18nPostprocess is not defined
        //
        // The fix: Changed to use ReadProp(i0.ɵɵi18nPostprocess) so the function
        // is properly accessed through the Angular core namespace import.
        let allocator = Allocator::default();

        // Create a simple input expression (simulating a $localize result)
        let input_expr = OutputExpression::Literal(oxc_allocator::Box::new_in(
            LiteralExpr {
                value: LiteralValue::String(Ident::from("test message")),
                source_span: None,
            },
            &allocator,
        ));

        // Call wrap_with_postprocess with no extra params
        let result = wrap_with_postprocess(&allocator, input_expr, &[]);

        // Emit the result to a string and verify
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result);

        // The output must contain "i0.ɵɵi18nPostprocess" (namespace-prefixed),
        // NOT a bare "ɵɵi18nPostprocess" without the i0. prefix.
        assert!(
            output.contains("i0.ɵɵi18nPostprocess"),
            "wrap_with_postprocess should emit i0.ɵɵi18nPostprocess (with namespace prefix), but got:\n{}",
            output
        );
    }

    #[test]
    fn test_wrap_with_postprocess_with_params_uses_namespace_prefix() {
        // Same as above but with postprocessing params to test the full path.
        let allocator = Allocator::default();

        let input_expr = OutputExpression::ReadVar(oxc_allocator::Box::new_in(
            ReadVarExpr { name: Ident::from("i18n_0"), source_span: None },
            &allocator,
        ));

        let params = vec![("ICU_0".to_string(), vec!["i18n_1".to_string(), "i18n_2".to_string()])];

        let result = wrap_with_postprocess(&allocator, input_expr, &params);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result);

        // Verify namespace prefix is present
        assert!(
            output.contains("i0.ɵɵi18nPostprocess"),
            "wrap_with_postprocess with params should emit i0.ɵɵi18nPostprocess, but got:\n{}",
            output
        );

        // Verify the function is called with the expression and the params map
        assert!(
            output.contains("i18n_0"),
            "Should contain the input expression, but got:\n{}",
            output
        );
    }
}
