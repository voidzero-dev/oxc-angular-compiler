//! Generate projection definitions phase.
//!
//! Generates projection definitions for ng-content elements.
//!
//! When a component uses `<ng-content>` for content projection,
//! we need to generate a `ɵɵprojectionDef()` instruction that
//! describes the projection slots and their selectors.
//!
//! Ported from Angular's `template/pipeline/src/phases/generate_projection_def.ts`.

use oxc_allocator::{Box as OxcBox, Vec as OxcVec};
use oxc_span::Ident;
use std::ptr::NonNull;

use crate::ir::ops::{CreateOp, CreateOpBase, ProjectionDefOp, XrefId};
use crate::output::ast::{LiteralArrayExpr, LiteralExpr, LiteralValue, OutputExpression};
use crate::pipeline::compilation::ComponentCompilationJob;
use crate::pipeline::selector::{parse_selector_to_r3_selector, r3_selector_to_output_expr};

/// Generates projection definitions for content projection.
///
/// This phase:
/// 1. Finds all Projection ops across all views
/// 2. Assigns each projection a unique ascending slot index
/// 3. Collects their selectors and converts to R3 format
/// 4. Sets job.content_selectors with the original selectors
/// 5. Creates a ProjectionDef op with R3 format selectors
pub fn generate_projection_defs(job: &mut ComponentCompilationJob<'_>) {
    // Phase 1: Collect all projection operation pointers across ALL views
    let mut projection_ptrs: Vec<NonNull<CreateOp<'_>>> = Vec::new();

    // Collect from root view using cursor
    {
        let mut cursor = job.root.create.cursor_front();
        loop {
            if let Some(op) = cursor.current() {
                if matches!(op, CreateOp::Projection(_)) {
                    if let Some(ptr) = cursor.current_ptr() {
                        projection_ptrs.push(ptr);
                    }
                }
            }
            if !cursor.move_next() {
                break;
            }
        }
    }

    // Collect from embedded views
    let view_xrefs: Vec<XrefId> = job.views.keys().copied().collect();
    for view_xref in view_xrefs {
        if let Some(view) = job.views.get_mut(&view_xref) {
            let mut cursor = view.create.cursor_front();
            loop {
                if let Some(op) = cursor.current() {
                    if matches!(op, CreateOp::Projection(_)) {
                        if let Some(ptr) = cursor.current_ptr() {
                            projection_ptrs.push(ptr);
                        }
                    }
                }
                if !cursor.move_next() {
                    break;
                }
            }
        }
    }

    if projection_ptrs.is_empty() {
        return;
    }

    // Phase 2: Assign projection slot indexes and collect selectors
    let allocator = job.allocator;
    let mut selectors: Vec<Ident<'_>> = Vec::new();

    for (slot_index, ptr) in projection_ptrs.iter().enumerate() {
        // SAFETY: ptr is valid from our cursor traversal
        let op = unsafe { &mut *ptr.as_ptr() };
        if let CreateOp::Projection(proj) = op {
            // Assign the projection slot index
            proj.projection_slot_index = slot_index as u32;

            // Collect selector (use "*" for wildcard if None)
            let selector = proj.selector.clone().unwrap_or_else(|| Ident::from("*"));
            selectors.push(selector);
        }
    }

    // Phase 3: Check if we need to create a ProjectionDef with explicit selectors
    // Only include selectors if there's more than one projection or the single one isn't a wildcard
    let needs_explicit_selectors = selectors.len() > 1
        || (selectors.len() == 1 && selectors.first().is_some_and(|s| s.as_str() != "*"));

    // Phase 4: Create and insert ProjectionDef at the beginning of root view
    // Check if there's already a ProjectionDef
    let has_projection_def =
        job.root.create.iter().any(|op| matches!(op, CreateOp::ProjectionDef(_)));

    // IMPORTANT: Order matters for constant pool indices!
    // TypeScript Angular creates defExpr FIRST, then contentSelectors SECOND.
    // This ensures constants are assigned indices in the correct order.

    // Create defExpr FIRST (to match TypeScript Angular's constant ordering)
    let def_expr = if needs_explicit_selectors {
        // Convert selectors to R3 format for projectionDef instruction
        // Build the full def array expression: [selector1, selector2, ...]
        // Where each selector is either "*" (string) or R3 format (array)
        let mut def_elements = OxcVec::with_capacity_in(selectors.len(), allocator);

        for selector in &selectors {
            if selector.as_str() == "*" {
                // Wildcard stays as string literal "*"
                def_elements.push(OutputExpression::Literal(OxcBox::new_in(
                    LiteralExpr {
                        value: LiteralValue::String(Ident::from("*")),
                        source_span: None,
                    },
                    allocator,
                )));
            } else {
                // Parse and convert to R3 format
                // parseSelectorToR3Selector returns an array of selector arrays (one per comma-separated selector)
                let r3_list = parse_selector_to_r3_selector(selector.as_str());

                // Build the selector list array - each R3 selector becomes a nested array
                // TypeScript: literalOrArrayLiteral(parseSelectorToR3Selector(s))
                // This preserves the nesting: [[["", "slot", "value"]]] for single selectors
                let mut selector_array = OxcVec::with_capacity_in(r3_list.len(), allocator);
                for r3_sel in &r3_list {
                    let inner_array = r3_selector_to_output_expr(allocator, r3_sel);
                    selector_array.push(OutputExpression::LiteralArray(OxcBox::new_in(
                        LiteralArrayExpr { entries: inner_array, source_span: None },
                        allocator,
                    )));
                }

                // Always wrap in an outer array to match TypeScript's literalOrArrayLiteral behavior
                // This ensures the correct nesting level: [[["", "slot", "value"]]]
                def_elements.push(OutputExpression::LiteralArray(OxcBox::new_in(
                    LiteralArrayExpr { entries: selector_array, source_span: None },
                    allocator,
                )));
            }
        }

        let literal_array = OutputExpression::LiteralArray(OxcBox::new_in(
            LiteralArrayExpr { entries: def_elements, source_span: None },
            allocator,
        ));
        // Pool the constant like Angular TS does with job.pool.getConstLiteral()
        Some(job.pool.get_const_literal(literal_array, true))
    } else {
        None
    };

    // Create contentSelectors SECOND (to match TypeScript Angular's constant ordering)
    {
        let mut selector_exprs = OxcVec::with_capacity_in(selectors.len(), allocator);
        for selector in &selectors {
            selector_exprs.push(OutputExpression::Literal(OxcBox::new_in(
                LiteralExpr { value: LiteralValue::String(selector.clone()), source_span: None },
                allocator,
            )));
        }
        let literal_array = OutputExpression::LiteralArray(OxcBox::new_in(
            LiteralArrayExpr { entries: selector_exprs, source_span: None },
            allocator,
        ));
        // Pool the constant like Angular TS does with job.pool.getConstLiteral()
        job.content_selectors = Some(job.pool.get_const_literal(literal_array, true));
    }

    // Insert ProjectionDef if not already present
    if !has_projection_def {
        let projection_def = CreateOp::ProjectionDef(ProjectionDefOp {
            base: CreateOpBase::default(),
            def: def_expr,
        });

        job.root.create.push_front(projection_def);
    }
}
