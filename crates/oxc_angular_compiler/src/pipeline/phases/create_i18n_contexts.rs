//! Create i18n contexts phase.
//!
//! Create one helper context op per i18n block (including generated descending blocks).
//!
//! Also, if an ICU exists inside an i18n block that also contains other localizable content
//! (such as string), create an additional helper context op for the ICU.
//!
//! These context ops are later used for generating i18n messages. (Although we generate at
//! least one context op per nested view, we will collect them up the tree later, to generate
//! a top-level message.)
//!
//! Ported from Angular's `template/pipeline/src/phases/create_i18n_contexts.ts`.

use oxc_span::Atom;
use rustc_hash::FxHashMap;

use crate::ir::enums::I18nContextKind;
use crate::ir::ops::{CreateOp, CreateOpBase, I18nContextOp, UpdateOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Creates i18n context ops for i18n message generation.
pub fn create_i18n_contexts(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Phase 1: Create i18n context ops for i18n attrs.
    // For attributes with i18n messages, we create an I18nContext op.
    //
    // The dedup key is the i18n message's instance_id (u32), matching Angular TS where
    // the Map key is the i18n.Message object reference (object identity).
    let mut attr_context_by_message: FxHashMap<u32, XrefId> = FxHashMap::default();
    let view_xrefs_for_attrs: Vec<XrefId> =
        std::iter::once(job.root.xref).chain(job.views.keys().copied()).collect();

    // Collect attribute ops that need i18n contexts
    #[derive(Clone)]
    struct AttrI18nInfo<'a> {
        view_xref: XrefId,
        message_instance_id: u32,
        is_create_op: bool, // true for ExtractedAttribute, false for update ops
        // Additional fields to uniquely identify the attribute
        target: XrefId, // Element xref this attribute belongs to
        name: Atom<'a>, // Attribute name
    }
    let mut attr_ops_needing_context: Vec<AttrI18nInfo<'_>> = Vec::new();

    for view_xref in &view_xrefs_for_attrs {
        let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(*view_xref) };

        if let Some(view) = view {
            // Check CreateOps (ExtractedAttribute)
            for op in view.create.iter() {
                if let CreateOp::ExtractedAttribute(attr_op) = op {
                    if let Some(instance_id) = attr_op.i18n_message {
                        attr_ops_needing_context.push(AttrI18nInfo {
                            view_xref: *view_xref,
                            message_instance_id: instance_id,
                            is_create_op: true,
                            target: attr_op.target,
                            name: attr_op.name.clone(),
                        });
                    }
                }
            }

            // Check UpdateOps (Property, Attribute)
            for op in view.update.iter() {
                match op {
                    UpdateOp::Property(prop_op) => {
                        if let Some(instance_id) = prop_op.i18n_message {
                            attr_ops_needing_context.push(AttrI18nInfo {
                                view_xref: *view_xref,
                                message_instance_id: instance_id,
                                is_create_op: false,
                                target: prop_op.target,
                                name: prop_op.name.clone(),
                            });
                        }
                    }
                    UpdateOp::Attribute(attr_op) => {
                        if let Some(instance_id) = attr_op.i18n_message {
                            attr_ops_needing_context.push(AttrI18nInfo {
                                view_xref: *view_xref,
                                message_instance_id: instance_id,
                                is_create_op: false,
                                target: attr_op.target,
                                name: attr_op.name.clone(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Create I18nContext ops keyed by i18n message instance_id.
    //
    // This matches TypeScript's behavior where `attrContextByMessage` uses the i18n.Message
    // object as the key. In TypeScript, when an attribute is copied (e.g., from element to
    // conditional via ingestControlFlowInsertionPoint), both uses share the same i18n.Message
    // object reference, so they get the same context.
    //
    // We use the instance_id as the key. Since the instance_id is assigned during parsing
    // and survives Rust moves/copies, attributes from the same source share the same
    // instance_id and get the same context here.
    for info in &attr_ops_needing_context {
        // Skip if we've already created a context for this message instance_id
        if attr_context_by_message.contains_key(&info.message_instance_id) {
            continue;
        }

        let context_xref = job.allocate_xref_id();
        let context_op = CreateOp::I18nContext(I18nContextOp {
            base: CreateOpBase::default(),
            xref: context_xref,
            context_kind: I18nContextKind::Attr,
            i18n_block: None, // Attribute contexts don't have an i18n block
            params: oxc_allocator::HashMap::new_in(allocator),
            postprocessing_params: oxc_allocator::HashMap::new_in(allocator),
            icu_placeholder_literals: oxc_allocator::HashMap::new_in(allocator),
            message: Some(info.message_instance_id),
        });

        // Add context op to the view
        if info.view_xref.0 == 0 {
            job.root.create.push(context_op);
        } else if let Some(view) = job.view_mut(info.view_xref) {
            view.create.push(context_op);
        }

        attr_context_by_message.insert(info.message_instance_id, context_xref);
    }

    // Assign contexts to the attribute ops using the message-based map
    for info in attr_ops_needing_context {
        if let Some(&context_xref) = attr_context_by_message.get(&info.message_instance_id) {
            let view = if info.view_xref.0 == 0 {
                Some(&mut job.root)
            } else {
                job.view_mut(info.view_xref)
            };

            if let Some(view) = view {
                if info.is_create_op {
                    // Update the specific ExtractedAttribute op (match by target and name)
                    for op in view.create.iter_mut() {
                        if let CreateOp::ExtractedAttribute(attr_op) = op {
                            if attr_op.target == info.target && attr_op.name == info.name {
                                attr_op.i18n_context = Some(context_xref);
                            }
                        }
                    }
                } else {
                    // Update the specific Property or Attribute op (match by target and name)
                    for op in view.update.iter_mut() {
                        match op {
                            UpdateOp::Property(prop_op) => {
                                if prop_op.target == info.target && prop_op.name == info.name {
                                    prop_op.i18n_context = Some(context_xref);
                                }
                            }
                            UpdateOp::Attribute(attr_op) => {
                                if attr_op.target == info.target && attr_op.name == info.name {
                                    attr_op.i18n_context = Some(context_xref);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    // Phase 2: Create i18n context ops for root i18n blocks.
    // A root i18n block is one where xref == root (or root is None).
    let mut block_context_by_i18n_block: FxHashMap<XrefId, XrefId> = FxHashMap::default();

    // First pass: collect root i18n blocks and create contexts for them
    let mut root_i18n_blocks: Vec<(XrefId, XrefId, Option<u32>)> = Vec::new(); // (view_xref, i18n_xref, message_instance_id)

    // Collect from root view
    {
        let root_xref = job.root.xref;
        for op in job.root.create.iter() {
            if let CreateOp::I18nStart(i18n_op) = op {
                // A root i18n block has root == None or root == Some(xref)
                let is_root = i18n_op.root.is_none() || i18n_op.root == Some(i18n_op.xref);
                if is_root {
                    root_i18n_blocks.push((root_xref, i18n_op.xref, i18n_op.message));
                }
            }
        }
    }

    // Collect from other views
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();
    for view_xref in &view_xrefs {
        if let Some(view) = job.view(*view_xref) {
            for op in view.create.iter() {
                if let CreateOp::I18nStart(i18n_op) = op {
                    let is_root = i18n_op.root.is_none() || i18n_op.root == Some(i18n_op.xref);
                    if is_root {
                        root_i18n_blocks.push((*view_xref, i18n_op.xref, i18n_op.message));
                    }
                }
            }
        }
    }

    // Create I18nContext ops for root i18n blocks
    for (view_xref, i18n_xref, message) in root_i18n_blocks {
        let context_xref = job.allocate_xref_id();

        let context_op = CreateOp::I18nContext(I18nContextOp {
            base: CreateOpBase::default(),
            xref: context_xref,
            context_kind: I18nContextKind::RootI18n,
            i18n_block: Some(i18n_xref),
            params: oxc_allocator::HashMap::new_in(allocator),
            postprocessing_params: oxc_allocator::HashMap::new_in(allocator),
            icu_placeholder_literals: oxc_allocator::HashMap::new_in(allocator),
            message,
        });

        // Add context op to the view
        if view_xref.0 == 0 {
            job.root.create.push(context_op);
        } else if let Some(view) = job.view_mut(view_xref) {
            view.create.push(context_op);
        }

        // Update the I18nStart op to reference this context
        if view_xref.0 == 0 {
            for op in job.root.create.iter_mut() {
                if let CreateOp::I18nStart(i18n_op) = op {
                    if i18n_op.xref == i18n_xref {
                        i18n_op.context = Some(context_xref);
                        break;
                    }
                }
            }
        } else if let Some(view) = job.view_mut(view_xref) {
            for op in view.create.iter_mut() {
                if let CreateOp::I18nStart(i18n_op) = op {
                    if i18n_op.xref == i18n_xref {
                        i18n_op.context = Some(context_xref);
                        break;
                    }
                }
            }
        }

        block_context_by_i18n_block.insert(i18n_xref, context_xref);
    }

    // Phase 3: Assign i18n contexts for child i18n blocks.
    // These don't need their own context, they inherit from their root i18n block.
    let mut child_i18n_blocks: Vec<(XrefId, XrefId, XrefId)> = Vec::new(); // (view_xref, i18n_xref, root_xref)

    // Collect from root view
    for op in job.root.create.iter() {
        if let CreateOp::I18nStart(i18n_op) = op {
            if let Some(root) = i18n_op.root {
                if root != i18n_op.xref {
                    child_i18n_blocks.push((job.root.xref, i18n_op.xref, root));
                }
            }
        }
    }

    // Collect from other views
    for view_xref in &view_xrefs {
        if let Some(view) = job.view(*view_xref) {
            for op in view.create.iter() {
                if let CreateOp::I18nStart(i18n_op) = op {
                    if let Some(root) = i18n_op.root {
                        if root != i18n_op.xref {
                            child_i18n_blocks.push((*view_xref, i18n_op.xref, root));
                        }
                    }
                }
            }
        }
    }

    // Assign contexts to child i18n blocks
    for (view_xref, i18n_xref, root_xref) in child_i18n_blocks {
        if let Some(&root_context) = block_context_by_i18n_block.get(&root_xref) {
            // Update the child I18nStart op to use the root's context
            if view_xref.0 == 0 {
                for op in job.root.create.iter_mut() {
                    if let CreateOp::I18nStart(i18n_op) = op {
                        if i18n_op.xref == i18n_xref {
                            i18n_op.context = Some(root_context);
                            break;
                        }
                    }
                }
            } else if let Some(view) = job.view_mut(view_xref) {
                for op in view.create.iter_mut() {
                    if let CreateOp::I18nStart(i18n_op) = op {
                        if i18n_op.xref == i18n_xref {
                            i18n_op.context = Some(root_context);
                            break;
                        }
                    }
                }
            }

            // Also track this child block's context for ICU handling
            block_context_by_i18n_block.insert(i18n_xref, root_context);
        }
    }

    // Phase 4: Create or assign i18n contexts for ICUs.
    create_icu_contexts_for_view(job, job.root.xref, &block_context_by_i18n_block);

    for view_xref in view_xrefs {
        create_icu_contexts_for_view(job, view_xref, &block_context_by_i18n_block);
    }
}

/// Creates ICU contexts for a single view.
fn create_icu_contexts_for_view(
    job: &mut ComponentCompilationJob<'_>,
    view_xref: XrefId,
    _block_context_by_i18n_block: &FxHashMap<XrefId, XrefId>,
) {
    let allocator = job.allocator;

    // Collect ICU info: (icu_xref, icu_message_id, current_i18n_xref, current_i18n_message_id, current_i18n_context)
    let mut icu_info: Vec<(XrefId, Option<u32>, XrefId, Option<u32>, Option<XrefId>)> = Vec::new();

    {
        let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(view_xref) };

        if let Some(view) = view {
            let mut current_i18n: Option<(XrefId, Option<u32>, Option<XrefId>)> = None; // (xref, message_instance_id, context)

            for op in view.create.iter() {
                match op {
                    CreateOp::I18nStart(i18n_op) => {
                        current_i18n = Some((i18n_op.xref, i18n_op.message, i18n_op.context));
                    }
                    CreateOp::I18nEnd(_) => {
                        current_i18n = None;
                    }
                    CreateOp::IcuStart(icu_op) => {
                        if let Some((i18n_xref, i18n_message, i18n_context)) = current_i18n {
                            icu_info.push((
                                icu_op.xref,
                                icu_op.message,
                                i18n_xref,
                                i18n_message,
                                i18n_context,
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Process ICUs
    for (icu_xref, icu_message, i18n_xref, i18n_message, i18n_context) in icu_info {
        // Check if ICU message is different from parent i18n message
        let icu_is_sub_message = icu_message != i18n_message;

        if icu_is_sub_message {
            // This ICU is a sub-message inside its parent i18n block message.
            // We need to give it its own context.
            let context_xref = job.allocate_xref_id();

            // Find the root i18n block for this ICU
            let root_xref = find_root_i18n_block(job, view_xref, i18n_xref).unwrap_or(i18n_xref);

            let context_op = CreateOp::I18nContext(I18nContextOp {
                base: CreateOpBase::default(),
                xref: context_xref,
                context_kind: I18nContextKind::Icu,
                i18n_block: Some(root_xref),
                params: oxc_allocator::HashMap::new_in(allocator),
                postprocessing_params: oxc_allocator::HashMap::new_in(allocator),
                icu_placeholder_literals: oxc_allocator::HashMap::new_in(allocator),
                message: icu_message,
            });

            // Add context op to the view
            if view_xref.0 == 0 {
                job.root.create.push(context_op);
            } else if let Some(view) = job.view_mut(view_xref) {
                view.create.push(context_op);
            }

            // Update the IcuStart op to reference this context
            if view_xref.0 == 0 {
                for op in job.root.create.iter_mut() {
                    if let CreateOp::IcuStart(icu_op) = op {
                        if icu_op.xref == icu_xref {
                            icu_op.context = Some(context_xref);
                            break;
                        }
                    }
                }
            } else if let Some(view) = job.view_mut(view_xref) {
                for op in view.create.iter_mut() {
                    if let CreateOp::IcuStart(icu_op) = op {
                        if icu_op.xref == icu_xref {
                            icu_op.context = Some(context_xref);
                            break;
                        }
                    }
                }
            }
        } else {
            // This ICU is the only translatable content in its parent i18n block.
            // We need to convert the parent's context into an ICU context.
            if let Some(context_xref) = i18n_context {
                // Update the IcuStart op to use the parent's context
                if view_xref.0 == 0 {
                    for op in job.root.create.iter_mut() {
                        if let CreateOp::IcuStart(icu_op) = op {
                            if icu_op.xref == icu_xref {
                                icu_op.context = Some(context_xref);
                                break;
                            }
                        }
                    }

                    // Update the context's kind to Icu
                    for op in job.root.create.iter_mut() {
                        if let CreateOp::I18nContext(ctx_op) = op {
                            if ctx_op.xref == context_xref {
                                ctx_op.context_kind = I18nContextKind::Icu;
                                break;
                            }
                        }
                    }
                } else if let Some(view) = job.view_mut(view_xref) {
                    for op in view.create.iter_mut() {
                        if let CreateOp::IcuStart(icu_op) = op {
                            if icu_op.xref == icu_xref {
                                icu_op.context = Some(context_xref);
                                break;
                            }
                        }
                    }

                    for op in view.create.iter_mut() {
                        if let CreateOp::I18nContext(ctx_op) = op {
                            if ctx_op.xref == context_xref {
                                ctx_op.context_kind = I18nContextKind::Icu;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Finds the root i18n block for a given i18n block.
fn find_root_i18n_block(
    job: &ComponentCompilationJob<'_>,
    view_xref: XrefId,
    i18n_xref: XrefId,
) -> Option<XrefId> {
    let view = if view_xref.0 == 0 { Some(&job.root) } else { job.view(view_xref) };

    if let Some(view) = view {
        for op in view.create.iter() {
            if let CreateOp::I18nStart(i18n_op) = op {
                if i18n_op.xref == i18n_xref {
                    return i18n_op.root;
                }
            }
        }
    }

    None
}
