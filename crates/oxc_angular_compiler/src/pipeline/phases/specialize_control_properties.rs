//! Specialize control properties phase.
//!
//! For each eligible control binding (`formField`, and on v22+ `formControl`,
//! `formControlName`, `ngModel`) in a view's update list, this phase:
//!   1. inserts a `ControlCreateOp` into the create list, anchored to the
//!      element/container create op that owns the binding target, and
//!   2. inserts an `UpdateOp::Control` right after the binding update op.
//!
//! Reification emits `ɵɵcontrolCreate()` / `ɵɵcontrol()` for these ops.
//!
//! Version differences (see `AngularVersion` predicates):
//!   - `< v21.2`: no control instructions at all.
//!   - `v21.2`–`v22.1.x`: `ɵɵcontrolCreate()` is anchored after the *last*
//!     create op for the target (`ElementEnd`/`ContainerEnd`/`Template` count),
//!     so it lands just after the element ends.
//!   - `v22.2+`: `ɵɵcontrolCreate()` is anchored after the *first* create op
//!     for the target (only `Element`/`ElementStart`/`Container`/`ContainerStart`
//!     count), so it lands right after the element start. `ng-template` stops
//!     being a valid anchor, which drops control instructions there entirely.
//!
//! Ported from Angular's `template/pipeline/src/phases/specialize_control_properties.ts`.

use std::ptr::NonNull;

use oxc_span::Span;
use oxc_str::Ident;

use crate::ast::r3::SecurityContext;
use crate::ir::enums::OpKind;
use crate::ir::list::{CreateOpList, UpdateOpList};
use crate::ir::ops::{
    ControlCreateOp, ControlOp, CreateOp, CreateOpBase, Op, UpdateOp, UpdateOpBase, XrefId,
};
use crate::pipeline::compilation::ComponentCompilationJob;

use super::binding_specialization::create_placeholder_expression;

/// Runs the specialize-control-properties phase for a template compilation job.
///
/// Corresponds to Angular's `specializeControlProperties` (template jobs only).
pub fn specialize_control_properties(job: &mut ComponentCompilationJob<'_>) {
    // Control instructions (`formField` controls) were introduced in v21.2.
    let Some(version) = job.angular_version else {
        // Unknown target: behave like the latest Angular.
        let extended = true;
        let first_anchor = true;
        run_job(job, extended, first_anchor);
        return;
    };
    if !version.supports_control_instructions() {
        return;
    }
    let extended = version.supports_extended_control_properties();
    let first_anchor = version.uses_first_control_anchor();
    run_job(job, extended, first_anchor);
}

fn run_job(job: &mut ComponentCompilationJob<'_>, extended: bool, first_anchor: bool) {
    let allocator = job.allocator;
    for view in job.all_views_mut() {
        process_view(&mut view.create, &mut view.update, allocator, extended, first_anchor);
    }
}

/// Metadata captured from an eligible update op.
struct Eligible<'a> {
    /// Pointer to the binding update op; `Control` is inserted after it.
    update_ptr: NonNull<UpdateOp<'a>>,
    /// Binding target (element/container/template xref).
    target: XrefId,
    /// Binding name (`formField`, `formControl`, `formControlName`, `ngModel`).
    name: Ident<'a>,
    /// Source span copied to both inserted ops.
    source_span: Option<Span>,
    /// Security context copied to the `Control` op.
    security_context: SecurityContext,
}

fn process_view<'a>(
    create: &mut CreateOpList<'a>,
    update: &mut UpdateOpList<'a>,
    allocator: &'a oxc_allocator::Allocator,
    extended: bool,
    first_anchor: bool,
) {
    // Collect eligible update ops first so we can mutate both lists freely.
    let mut eligible: Vec<Eligible<'a>> = Vec::new();
    let mut ptr = update.head_ptr();
    while let Some(op_ptr) = ptr {
        // SAFETY: pointers come from a live op list and outlive this loop.
        let op = unsafe { op_ptr.as_ref() };
        if let Some(e) = eligible_entry(op_ptr, op, extended) {
            eligible.push(e);
        }
        ptr = op.next();
    }

    for entry in eligible {
        let Some(anchor) = find_control_anchor(create, entry.target, first_anchor) else {
            // v22+: no valid anchor (e.g. `ng-template` targets) drops both
            // instructions. v21.2 upstream throws; we skip instead.
            continue;
        };
        let span = entry.source_span;
        // SAFETY: `anchor` is a live node of `create`; inserting after it keeps
        // the list well-formed.
        unsafe {
            create.insert_after(
                anchor,
                CreateOp::ControlCreate(ControlCreateOp {
                    base: CreateOpBase { source_span: span, ..Default::default() },
                }),
            );
        }
        // SAFETY: `entry.update_ptr` is a live node of `update`.
        unsafe {
            update.insert_after(
                entry.update_ptr,
                UpdateOp::Control(ControlOp {
                    base: UpdateOpBase { source_span: span, ..Default::default() },
                    target: entry.target,
                    name: entry.name,
                    expression: create_placeholder_expression(allocator),
                    security_context: entry.security_context,
                }),
            );
        }
    }
}

/// Returns an `Eligible` entry when `op` is a control-eligible binding.
///
/// Mirrors Angular's `ELIGIBLE_CONTROL_PROPERTIES`:
///   - `formField`: Property
///   - `formControl`: Property (v22+)
///   - `formControlName`: Property or Attribute (v22+)
///   - `ngModel`: Attribute, Property, or TwoWayProperty (v22+)
fn eligible_entry<'a>(
    update_ptr: NonNull<UpdateOp<'a>>,
    op: &UpdateOp<'a>,
    extended: bool,
) -> Option<Eligible<'a>> {
    let kind = op.kind();
    let (name, target, source_span, security_context) = match op {
        UpdateOp::Property(p) => (p.name, p.target, p.base.source_span, p.security_context),
        UpdateOp::Attribute(a) => (a.name, a.target, a.base.source_span, a.security_context),
        UpdateOp::TwoWayProperty(t) => (t.name, t.target, t.base.source_span, t.security_context),
        _ => return None,
    };
    let eligible = match name.as_str() {
        "formField" => kind == OpKind::Property,
        "formControl" => extended && kind == OpKind::Property,
        "formControlName" => extended && matches!(kind, OpKind::Property | OpKind::Attribute),
        "ngModel" => {
            extended
                && matches!(kind, OpKind::Attribute | OpKind::Property | OpKind::TwoWayProperty)
        }
        _ => false,
    };
    eligible.then_some(Eligible { update_ptr, target, name, source_span, security_context })
}

/// Finds the create-op anchor for `ɵɵcontrolCreate()`.
///
/// Pre-v22.2 upstream keeps the *last* relevant create op for the target
/// (element/container ends and templates included). v22.2+ keeps the *first*
/// and only accepts element/container starts (or their collapsed forms).
fn find_control_anchor<'a>(
    create: &CreateOpList<'a>,
    target: XrefId,
    first_anchor: bool,
) -> Option<NonNull<CreateOp<'a>>> {
    let mut last: Option<NonNull<CreateOp<'a>>> = None;
    let mut ptr = create.head_ptr();
    while let Some(op_ptr) = ptr {
        // SAFETY: pointers come from a live op list and outlive this loop.
        let op = unsafe { op_ptr.as_ref() };
        if let Some(xref) = anchor_xref(op, first_anchor)
            && xref == target
        {
            if first_anchor {
                return Some(op_ptr);
            }
            last = Some(op_ptr);
        }

        ptr = op.next();
    }
    last
}

/// Returns the xref of a create op that can anchor `ɵɵcontrolCreate()`,
/// restricted to the version-appropriate kind set.
fn anchor_xref(op: &CreateOp<'_>, first_anchor: bool) -> Option<XrefId> {
    match op {
        CreateOp::Element(e) => Some(e.xref),
        CreateOp::ElementStart(e) => Some(e.xref),
        CreateOp::Container(c) => Some(c.xref),
        CreateOp::ContainerStart(c) => Some(c.xref),
        CreateOp::ElementEnd(e) if !first_anchor => Some(e.xref),
        CreateOp::ContainerEnd(c) if !first_anchor => Some(c.xref),
        CreateOp::Template(t) if !first_anchor => Some(t.xref),
        _ => None,
    }
}
