//! Element, container, template, and text statement generation.

use oxc_allocator::{Box, Vec as OxcVec};
use oxc_span::Ident;

use crate::output::ast::{
    LiteralExpr, LiteralValue, OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;

use super::super::utils::create_instruction_call_stmt;

/// Creates arguments for element instructions.
///
/// Arguments: slot, tag, [constIndex], [localRefIndex]
/// If localRefIndex is present, constIndex must also be present (even if null).
pub fn create_element_args<'a>(
    allocator: &'a oxc_allocator::Allocator,
    tag: &Ident<'a>,
    slot: u32,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OxcVec<'a, OutputExpression<'a>> {
    let mut args = OxcVec::new_in(allocator);
    // Slot index
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        allocator,
    )));
    // Tag name
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(tag.clone()), source_span: None },
        allocator,
    )));
    // If localRefIndex is present, we need both constIndex and localRefIndex
    if let Some(refs_idx) = local_refs_index {
        // Push constIndex (might be null)
        if let Some(attr_idx) = attributes {
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Number(attr_idx as f64), source_span: None },
                allocator,
            )));
        } else {
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Null, source_span: None },
                allocator,
            )));
        }
        // Push localRefIndex
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(refs_idx as f64), source_span: None },
            allocator,
        )));
    } else if let Some(attr_idx) = attributes {
        // Only constIndex, no localRefIndex
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(attr_idx as f64), source_span: None },
            allocator,
        )));
    }
    args
}

/// Creates an ɵɵelementStart() call statement.
pub fn create_element_start_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    tag: &Ident<'a>,
    slot: u32,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OutputStatement<'a> {
    let args = create_element_args(allocator, tag, slot, attributes, local_refs_index);
    create_instruction_call_stmt(allocator, Identifiers::ELEMENT_START, args)
}

/// Creates an ɵɵelement() call statement.
pub fn create_element_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    tag: &Ident<'a>,
    slot: u32,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OutputStatement<'a> {
    let args = create_element_args(allocator, tag, slot, attributes, local_refs_index);
    create_instruction_call_stmt(allocator, Identifiers::ELEMENT, args)
}

/// Creates an ɵɵelementEnd() call statement.
pub fn create_element_end_stmt<'a>(allocator: &'a oxc_allocator::Allocator) -> OutputStatement<'a> {
    create_instruction_call_stmt(allocator, Identifiers::ELEMENT_END, OxcVec::new_in(allocator))
}

// =============================================================================
// DOM-only element instructions (for DomOnly compilation mode)
// =============================================================================

/// Creates an ɵɵdomElementStart() call statement.
///
/// Used in DomOnly mode when the component has no directive dependencies.
/// This is an optimized version that skips directive matching at runtime.
pub fn create_dom_element_start_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    tag: &Ident<'a>,
    slot: u32,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OutputStatement<'a> {
    let args = create_element_args(allocator, tag, slot, attributes, local_refs_index);
    create_instruction_call_stmt(allocator, Identifiers::DOM_ELEMENT_START, args)
}

/// Creates an ɵɵdomElement() call statement.
///
/// Used in DomOnly mode when the component has no directive dependencies.
/// This is an optimized version that skips directive matching at runtime.
pub fn create_dom_element_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    tag: &Ident<'a>,
    slot: u32,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OutputStatement<'a> {
    let args = create_element_args(allocator, tag, slot, attributes, local_refs_index);
    create_instruction_call_stmt(allocator, Identifiers::DOM_ELEMENT, args)
}

/// Creates an ɵɵdomElementEnd() call statement.
///
/// Used in DomOnly mode when the component has no directive dependencies.
pub fn create_dom_element_end_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
) -> OutputStatement<'a> {
    create_instruction_call_stmt(allocator, Identifiers::DOM_ELEMENT_END, OxcVec::new_in(allocator))
}

/// Creates an ɵɵtext() call statement.
///
/// The ɵɵtext instruction takes:
/// - slot: The slot index for the text node
/// - initial_value: Optional initial text content (for static text)
pub fn create_text_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    initial_value: Option<&'a str>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        allocator,
    )));

    // Add initial text value if present and non-empty
    if let Some(value) = initial_value {
        if !value.is_empty() {
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(value.into()), source_span: None },
                allocator,
            )));
        }
    }

    create_instruction_call_stmt(allocator, Identifiers::TEXT, args)
}

/// Creates an ɵɵtemplate() call statement.
///
/// The ɵɵtemplate instruction takes:
/// - slot: The slot index for the template
/// - templateFn: Reference to the template function
/// - decls: Number of declarations
/// - vars: Number of variables
/// - tag: Optional HTML tag name (for content projection)
/// - attributes: Optional const array index for attributes
/// - localRefs: Optional const array index for local refs (if present, also adds templateRefExtractor)
pub fn create_template_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    fn_name: Option<Ident<'a>>,
    decls: Option<u32>,
    vars: Option<u32>,
    tag: Option<&Ident<'a>>,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OutputStatement<'a> {
    let args = create_template_args(
        allocator,
        slot,
        fn_name,
        decls,
        vars,
        tag,
        attributes,
        local_refs_index,
    );
    create_instruction_call_stmt(allocator, Identifiers::TEMPLATE_CREATE, args)
}

/// Creates arguments for template instructions (shared between template and domTemplate).
///
/// Arguments: slot, templateFnRef, decls, vars, tag, constIndex, [localRefs, templateRefExtractor]
/// Trailing null arguments are stripped.
fn create_template_args<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    fn_name: Option<Ident<'a>>,
    decls: Option<u32>,
    vars: Option<u32>,
    tag: Option<&Ident<'a>>,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OxcVec<'a, OutputExpression<'a>> {
    let mut args = OxcVec::new_in(allocator);

    // Slot index
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        allocator,
    )));

    // Template function reference
    if let Some(name) = fn_name {
        args.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name, source_span: None },
            allocator,
        )));
    } else {
        let placeholder_str = allocator.alloc_str(&format!("_r{slot}"));
        let placeholder = Ident::from(placeholder_str);
        args.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: placeholder, source_span: None },
            allocator,
        )));
    }

    // Declaration count
    let decl_count = decls.unwrap_or(0);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(decl_count as f64), source_span: None },
        allocator,
    )));

    // Variable count
    let var_count = vars.unwrap_or(0);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(var_count as f64), source_span: None },
        allocator,
    )));

    // Tag (string | null)
    if let Some(t) = tag {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(t.clone()), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Attributes const index (number | null)
    if let Some(attr_idx) = attributes {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(attr_idx as f64), source_span: None },
            allocator,
        )));
    } else {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )));
    }

    // Local refs index and templateRefExtractor
    if let Some(refs_idx) = local_refs_index {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(refs_idx as f64), source_span: None },
            allocator,
        )));
        // Add templateRefExtractor: i0.ɵɵtemplateRefExtractor
        args.push(OutputExpression::ReadProp(Box::new_in(
            ReadPropExpr {
                receiver: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: Ident::from("i0"), source_span: None },
                        allocator,
                    )),
                    allocator,
                ),
                name: Ident::from(Identifiers::TEMPLATE_REF_EXTRACTOR),
                optional: false,
                source_span: None,
            },
            allocator,
        )));
    }

    // Strip trailing null arguments (Angular compiler optimization)
    while !args.is_empty() {
        if let Some(OutputExpression::Literal(lit)) = args.last() {
            if matches!(lit.value, LiteralValue::Null) {
                args.pop();
                continue;
            }
        }
        break;
    }

    args
}

/// Creates an ɵɵdomTemplate() call statement.
///
/// Used in DomOnly mode for block templates (like @if, @for, @switch).
/// This is an optimized version that skips directive matching at runtime.
///
/// The ɵɵdomTemplate instruction takes:
/// - slot: The slot index for the template
/// - templateFn: Reference to the template function
/// - decls: Number of declarations
/// - vars: Number of variables
/// - tag: Optional HTML tag name (for content projection)
/// - attributes: Optional const array index for attributes
/// - localRefs: Optional const array index for local refs (if present, also adds templateRefExtractor)
pub fn create_dom_template_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    fn_name: Option<Ident<'a>>,
    decls: Option<u32>,
    vars: Option<u32>,
    tag: Option<&Ident<'a>>,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OutputStatement<'a> {
    let args = create_template_args(
        allocator,
        slot,
        fn_name,
        decls,
        vars,
        tag,
        attributes,
        local_refs_index,
    );
    create_instruction_call_stmt(allocator, Identifiers::DOM_TEMPLATE, args)
}

/// Creates arguments for container instructions.
///
/// Arguments: slot, [constIndex], [localRefIndex]
/// If localRefIndex is present, constIndex must also be present (even if null).
fn create_container_args<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OxcVec<'a, OutputExpression<'a>> {
    let mut args = OxcVec::new_in(allocator);
    // Slot index
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        allocator,
    )));
    // If localRefIndex is present, we need both constIndex and localRefIndex
    if let Some(refs_idx) = local_refs_index {
        // Push constIndex (might be null)
        if let Some(attr_idx) = attributes {
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Number(attr_idx as f64), source_span: None },
                allocator,
            )));
        } else {
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Null, source_span: None },
                allocator,
            )));
        }
        // Push localRefIndex
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(refs_idx as f64), source_span: None },
            allocator,
        )));
    } else if let Some(attr_idx) = attributes {
        // Only constIndex, no localRefIndex
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(attr_idx as f64), source_span: None },
            allocator,
        )));
    }
    args
}

/// Creates an ɵɵcontainer() call statement.
pub fn create_container_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OutputStatement<'a> {
    let args = create_container_args(allocator, slot, attributes, local_refs_index);
    create_instruction_call_stmt(allocator, Identifiers::ELEMENT_CONTAINER, args)
}

/// Creates an ɵɵcontainerEnd() call statement.
pub fn create_container_end_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
) -> OutputStatement<'a> {
    create_instruction_call_stmt(
        allocator,
        Identifiers::ELEMENT_CONTAINER_END,
        OxcVec::new_in(allocator),
    )
}

/// Creates an ɵɵelementContainerStart() call statement.
pub fn create_container_start_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OutputStatement<'a> {
    let args = create_container_args(allocator, slot, attributes, local_refs_index);
    create_instruction_call_stmt(allocator, Identifiers::ELEMENT_CONTAINER_START, args)
}

// =============================================================================
// DOM-only container instructions (for DomOnly compilation mode)
// =============================================================================

/// Creates an ɵɵdomElementContainer() call statement.
///
/// Used in DomOnly mode for ng-container elements.
pub fn create_dom_container_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OutputStatement<'a> {
    let args = create_container_args(allocator, slot, attributes, local_refs_index);
    create_instruction_call_stmt(allocator, Identifiers::DOM_ELEMENT_CONTAINER, args)
}

/// Creates an ɵɵdomElementContainerStart() call statement.
///
/// Used in DomOnly mode for ng-container elements.
pub fn create_dom_container_start_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    attributes: Option<u32>,
    local_refs_index: Option<u32>,
) -> OutputStatement<'a> {
    let args = create_container_args(allocator, slot, attributes, local_refs_index);
    create_instruction_call_stmt(allocator, Identifiers::DOM_ELEMENT_CONTAINER_START, args)
}

/// Creates an ɵɵdomElementContainerEnd() call statement.
///
/// Used in DomOnly mode for ng-container elements.
pub fn create_dom_container_end_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
) -> OutputStatement<'a> {
    create_instruction_call_stmt(
        allocator,
        Identifiers::DOM_ELEMENT_CONTAINER_END,
        OxcVec::new_in(allocator),
    )
}

/// Creates an ɵɵprojection() call statement.
///
/// # Arguments
/// * `slot` - The slot index for this projection
/// * `projection_slot_index` - The projection def slot index (which selector group this ng-content belongs to)
/// * `attributes` - Optional attributes array expression (for ng-content with attributes)
/// * `fallback_fn_name` - Optional fallback view function name (for ng-content with fallback content)
/// * `fallback_decls` - Optional fallback view declaration count
/// * `fallback_vars` - Optional fallback view variable count
pub fn create_projection_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    slot: u32,
    projection_slot_index: u32,
    attributes: Option<OutputExpression<'a>>,
    fallback_fn_name: Option<&'a str>,
    fallback_decls: Option<u32>,
    fallback_vars: Option<u32>,
) -> OutputStatement<'a> {
    let mut args = OxcVec::new_in(allocator);

    // First arg is always the slot
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(slot as f64), source_span: None },
        allocator,
    )));

    // Only add additional args if needed (projectionSlotIndex !== 0 || attributes || fallback)
    if projection_slot_index != 0 || attributes.is_some() || fallback_fn_name.is_some() {
        // Add projection slot index
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr {
                value: LiteralValue::Number(projection_slot_index as f64),
                source_span: None,
            },
            allocator,
        )));

        // Add attributes if present
        if let Some(attr_expr) = attributes {
            args.push(attr_expr);
        }

        // Add fallback args if present
        if let Some(fn_name) = fallback_fn_name {
            // If no attributes, add null placeholder
            if args.len() == 2 {
                args.push(OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Null, source_span: None },
                    allocator,
                )));
            }

            // Add fallback function name as variable reference
            args.push(OutputExpression::ReadVar(Box::new_in(
                crate::output::ast::ReadVarExpr { name: fn_name.into(), source_span: None },
                allocator,
            )));

            // Add fallback decls
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(fallback_decls.unwrap_or(0) as f64),
                    source_span: None,
                },
                allocator,
            )));

            // Add fallback vars
            args.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(fallback_vars.unwrap_or(0) as f64),
                    source_span: None,
                },
                allocator,
            )));
        }
    }

    create_instruction_call_stmt(allocator, Identifiers::PROJECTION, args)
}

/// Creates a namespace change statement.
pub fn create_namespace_stmt<'a>(
    allocator: &'a oxc_allocator::Allocator,
    namespace: crate::ir::enums::Namespace,
) -> OutputStatement<'a> {
    let instruction = match namespace {
        crate::ir::enums::Namespace::Html => Identifiers::NAMESPACE_HTML,
        crate::ir::enums::Namespace::Svg => Identifiers::NAMESPACE_SVG,
        crate::ir::enums::Namespace::Math => Identifiers::NAMESPACE_MATH_ML,
    };
    create_instruction_call_stmt(allocator, instruction, OxcVec::new_in(allocator))
}
