//! Local refs phase.
//!
//! Lifts local reference declarations on element-like structures within each view
//! into an entry in the `consts` array for the whole component.
//!
//! Template references like `<div #myDiv>` create local variables that can be
//! used to reference the element or directive. This phase serializes these
//! references to the const array.
//!
//! TypeScript Angular serializes local refs as a FLAT array:
//! ```javascript
//! ["name1", "target1", "name2", "target2", ...]
//! ```
//!
//! The arrays are placed INLINE in the `consts` property of `defineComponent()`,
//! NOT extracted to top-level const declarations.
//!
//! Ported from Angular's `template/pipeline/src/phases/local_refs.ts`.

use oxc_allocator::Vec as OxcVec;
use rustc_hash::FxHashMap;

use crate::ir::ops::{CreateOp, LocalRef, XrefId};
use crate::pipeline::compilation::{ComponentCompilationJob, ConstValue};

/// Info about an op that needs local refs serialized.
struct LocalRefInfo<'a> {
    op_kind: LocalRefOpKind,
    refs: std::vec::Vec<(oxc_span::Ident<'a>, oxc_span::Ident<'a>)>,
}

#[derive(Clone, Copy)]
enum LocalRefOpKind {
    ElementStart(XrefId),
    Element(XrefId),
    Template(XrefId),
    Conditional(XrefId),
    ConditionalBranch(XrefId),
    ContainerStart(XrefId),
    Container(XrefId),
}

/// Lifts local reference declarations on element-like structures within each view
/// into an entry in the `consts` array for the whole component.
///
/// This phase:
/// 1. Finds all local reference declarations (#ref on elements)
/// 2. Serializes them to FLAT array: [name1, target1, name2, target2, ...]
/// 3. Adds array INLINE to job.consts (NOT pooled to top-level const)
/// 4. Sets local_refs_index on the op to reference the const
pub fn lift_local_refs(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // First pass: Collect all ops with local refs (immutable borrow)
    let mut ops_with_refs: std::vec::Vec<LocalRefInfo<'_>> = std::vec::Vec::new();

    for view in job.all_views() {
        for op in view.create.iter() {
            let (op_kind, refs): (Option<LocalRefOpKind>, &[LocalRef<'_>]) = match op {
                CreateOp::ElementStart(el) => {
                    (Some(LocalRefOpKind::ElementStart(el.xref)), &el.local_refs)
                }
                CreateOp::Element(el) => (Some(LocalRefOpKind::Element(el.xref)), &el.local_refs),
                CreateOp::Template(tmpl) => {
                    (Some(LocalRefOpKind::Template(tmpl.xref)), &tmpl.local_refs)
                }
                CreateOp::Conditional(cond) => {
                    (Some(LocalRefOpKind::Conditional(cond.xref)), &cond.local_refs)
                }
                CreateOp::ConditionalBranch(branch) => {
                    (Some(LocalRefOpKind::ConditionalBranch(branch.xref)), &branch.local_refs)
                }
                CreateOp::ContainerStart(container) => {
                    (Some(LocalRefOpKind::ContainerStart(container.xref)), &container.local_refs)
                }
                CreateOp::Container(container) => {
                    (Some(LocalRefOpKind::Container(container.xref)), &container.local_refs)
                }
                _ => (None, &[]),
            };

            if let Some(kind) = op_kind {
                if !refs.is_empty() {
                    let refs_copy: std::vec::Vec<_> =
                        refs.iter().map(|r| (r.name.clone(), r.target.clone())).collect();
                    ops_with_refs.push(LocalRefInfo { op_kind: kind, refs: refs_copy });
                }
            }
        }
    }

    // Second pass: Serialize and add consts, building a map of xref -> const_idx
    let mut const_indices: FxHashMap<XrefId, u32> = FxHashMap::default();

    for info in &ops_with_refs {
        let xref = match info.op_kind {
            LocalRefOpKind::ElementStart(x)
            | LocalRefOpKind::Element(x)
            | LocalRefOpKind::Template(x)
            | LocalRefOpKind::Conditional(x)
            | LocalRefOpKind::ConditionalBranch(x)
            | LocalRefOpKind::ContainerStart(x)
            | LocalRefOpKind::Container(x) => x,
        };

        // Serialize refs to a ConstValue::Array: [name1, target1, name2, target2, ...]
        // Unlike pure functions, local refs are NOT pooled to top-level const declarations.
        // They are placed inline in the `consts` property of `defineComponent()`.
        let local_refs_const = serialize_local_refs_to_const_value(allocator, &info.refs);

        // Add the array directly to job.consts (no pooling)
        let const_idx = job.add_const(local_refs_const);
        const_indices.insert(xref, const_idx);
    }

    // Third pass: Apply const indices to ops (mutable borrow)
    let view_xrefs: std::vec::Vec<_> = job.all_views().map(|v| v.xref).collect();

    for view_xref in view_xrefs {
        if let Some(view) = job.view_mut(view_xref) {
            for op in view.create.iter_mut() {
                match op {
                    CreateOp::ElementStart(el) => {
                        if let Some(&idx) = const_indices.get(&el.xref) {
                            el.local_refs_index = Some(idx);
                        }
                    }
                    CreateOp::Element(el) => {
                        if let Some(&idx) = const_indices.get(&el.xref) {
                            el.local_refs_index = Some(idx);
                        }
                    }
                    CreateOp::Template(tmpl) => {
                        if let Some(&idx) = const_indices.get(&tmpl.xref) {
                            tmpl.local_refs_index = Some(idx);
                        }
                    }
                    CreateOp::Conditional(cond) => {
                        if let Some(&idx) = const_indices.get(&cond.xref) {
                            cond.local_refs_index = Some(idx);
                        }
                    }
                    CreateOp::ConditionalBranch(branch) => {
                        if let Some(&idx) = const_indices.get(&branch.xref) {
                            branch.local_refs_index = Some(idx);
                        }
                    }
                    CreateOp::ContainerStart(container) => {
                        if let Some(&idx) = const_indices.get(&container.xref) {
                            container.local_refs_index = Some(idx);
                        }
                    }
                    CreateOp::Container(container) => {
                        if let Some(&idx) = const_indices.get(&container.xref) {
                            container.local_refs_index = Some(idx);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Serializes local refs from tuple pairs to a ConstValue::Array.
///
/// Format: [name1, target1, name2, target2, ...]
/// This matches TypeScript Angular's `serializeLocalRefs` function.
///
/// The resulting array is placed INLINE in the `consts` property,
/// not extracted to a top-level const declaration.
fn serialize_local_refs_to_const_value<'a>(
    allocator: &'a oxc_allocator::Allocator,
    refs: &[(oxc_span::Ident<'a>, oxc_span::Ident<'a>)],
) -> ConstValue<'a> {
    let mut entries = OxcVec::with_capacity_in(refs.len() * 2, allocator);

    for (name, target) in refs {
        // Add name
        entries.push(ConstValue::String(name.clone()));

        // Add target (empty string if no explicit target)
        entries.push(ConstValue::String(target.clone()));
    }

    ConstValue::Array(entries)
}
