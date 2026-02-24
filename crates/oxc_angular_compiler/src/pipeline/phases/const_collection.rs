//! Const collection phase.
//!
//! Collects element constants into the consts array. This phase:
//! 1. Gathers all ExtractedAttribute operations by their target element
//! 2. Groups attributes by kind (attribute, class, style, binding, etc.)
//! 3. Serializes them into const array expressions
//! 4. Stores the arrays in the constant pool
//!
//! Ported from Angular's `template/pipeline/src/phases/const_collection.ts`.

use oxc_allocator::Vec as OxcVec;
use oxc_diagnostics::OxcDiagnostic;
use oxc_span::Atom;
use rustc_hash::FxHashMap;

use oxc_allocator::Box;

use crate::ir::enums::{BindingKind, CompatibilityMode};
use crate::ir::expression::IrExpression;
use crate::ir::ops::{CreateOp, ExtractedAttributeOp, XrefId};
use crate::output::ast::{LiteralArrayExpr, LiteralExpr, LiteralValue, OutputExpression};
use crate::pipeline::compilation::{
    ComponentCompilationJob, ConstValue, HostBindingCompilationJob,
};
use crate::pipeline::selector::{parse_selector_to_r3_selector, r3_selector_to_output_expr};

/// Represents an attribute value that can be either a string literal or an i18n variable reference.
///
/// This is needed to support i18n attributes where the value is a reference to an i18n
/// variable (e.g., I18N_0) rather than a literal string.
#[derive(Debug, Clone)]
enum AttributeValue<'a> {
    /// A string literal value.
    String(Atom<'a>),
    /// An i18n variable reference (stores just the variable name for efficient comparison).
    /// This will be serialized as a ReadVar expression in the const array.
    I18nVar(Atom<'a>),
}

/// Marker values for attribute arrays (matches Angular's AttributeMarker enum).
#[derive(Debug, Clone, Copy)]
#[repr(i32)]
enum AttributeMarker {
    /// Marker for namespace URIs.
    NamespaceUri = 0,
    /// Marker for class bindings.
    Classes = 1,
    /// Marker for style bindings.
    Styles = 2,
    /// Marker for property bindings.
    Bindings = 3,
    /// Marker for template attributes.
    Template = 4,
    /// Marker for ngProjectAs projection.
    ProjectAs = 5,
    /// Marker for i18n attributes.
    I18n = 6,
}

/// Container for element attributes grouped by kind.
///
/// Ported from Angular's ElementAttributes class in const_collection.ts.
/// Tracks known attributes to avoid duplicates, following Angular's behavior
/// where duplicate attribute, class, and style values are only allowed in
/// TemplateDefinitionBuilder compatibility mode.
struct ElementAttributes<'a> {
    /// Compatibility mode - determines whether duplicates are allowed.
    compatibility: CompatibilityMode,
    /// Tracks known (kind, name) pairs to prevent duplicates.
    /// Ported from Angular's `private known = new Map<ir.BindingKind, Set<string>>();`
    known: std::collections::HashMap<BindingKind, std::collections::HashSet<String>>,
    /// Static attributes (namespace, name, value).
    /// Value can be a string literal or an expression (for i18n variable references).
    attributes: std::vec::Vec<(Option<Atom<'a>>, Atom<'a>, Option<AttributeValue<'a>>)>,
    /// Class names.
    classes: std::vec::Vec<Atom<'a>>,
    /// Style properties.
    styles: std::vec::Vec<(Atom<'a>, Option<Atom<'a>>)>,
    /// Property bindings (just names for the const array).
    bindings: std::vec::Vec<Atom<'a>>,
    /// Template bindings.
    template: std::vec::Vec<Atom<'a>>,
    /// i18n attributes.
    i18n: std::vec::Vec<Atom<'a>>,
    /// The ngProjectAs selector value (if present).
    project_as: Option<Atom<'a>>,
}

impl<'a> ElementAttributes<'a> {
    /// Creates a new ElementAttributes with the specified compatibility mode.
    fn new(compatibility: CompatibilityMode) -> Self {
        Self {
            compatibility,
            known: std::collections::HashMap::default(),
            attributes: std::vec::Vec::new(),
            classes: std::vec::Vec::new(),
            styles: std::vec::Vec::new(),
            bindings: std::vec::Vec::new(),
            template: std::vec::Vec::new(),
            i18n: std::vec::Vec::new(),
            project_as: None,
        }
    }

    /// Checks if an attribute with the given kind and name has already been added.
    /// If not, marks it as known and returns false. If already known, returns true.
    ///
    /// Ported from Angular's `isKnown` method in const_collection.ts:
    /// ```typescript
    /// private isKnown(kind: ir.BindingKind, name: string) {
    ///   const nameToValue = this.known.get(kind) ?? new Set<string>();
    ///   this.known.set(kind, nameToValue);
    ///   if (nameToValue.has(name)) {
    ///     return true;
    ///   }
    ///   nameToValue.add(name);
    ///   return false;
    /// }
    /// ```
    fn is_known(&mut self, kind: BindingKind, name: &str) -> bool {
        let name_set = self.known.entry(kind).or_default();
        if name_set.contains(name) {
            return true;
        }
        name_set.insert(name.to_string());
        false
    }

    fn add(&mut self, attr: &ExtractedAttributeOp<'a>) {
        // TemplateDefinitionBuilder puts duplicate attribute, class, and style values into the consts
        // array. This seems inefficient, we can probably keep just the first one or the last value
        // (whichever actually gets applied when multiple values are listed for the same attribute).
        //
        // Ported from Angular's const_collection.ts lines 163-170:
        // const allowDuplicates =
        //   this.compatibility === ir.CompatibilityMode.TemplateDefinitionBuilder &&
        //   (kind === ir.BindingKind.Attribute ||
        //    kind === ir.BindingKind.ClassName ||
        //    kind === ir.BindingKind.StyleProperty);
        // if (!allowDuplicates && this.isKnown(kind, name)) {
        //   return;
        // }
        let allow_duplicates = self.compatibility == CompatibilityMode::TemplateDefinitionBuilder
            && matches!(
                attr.binding_kind,
                BindingKind::Attribute | BindingKind::ClassName | BindingKind::StyleProperty
            );
        if !allow_duplicates && self.is_known(attr.binding_kind, attr.name.as_str()) {
            return;
        }

        match attr.binding_kind {
            BindingKind::Attribute => {
                // For attributes, we store namespace, name, and value
                // Extract the actual value from the expression if available
                let value = Self::extract_attribute_value(attr);

                // Check for ngProjectAs attribute - store its value for special handling
                // ngProjectAs must have a string literal value, not an i18n variable reference
                if attr.name.as_str() == "ngProjectAs" {
                    if let Some(AttributeValue::String(ref val)) = value {
                        self.project_as = Some(val.clone());
                    }
                }

                self.attributes.push((attr.namespace.clone(), attr.name.clone(), value));
            }
            BindingKind::ClassName => {
                self.classes.push(attr.name.clone());
            }
            BindingKind::StyleProperty => {
                // Styles only support string values, not i18n variable references
                let value = Self::extract_attribute_value(attr);
                let string_value = match value {
                    Some(AttributeValue::String(s)) => Some(s),
                    _ => None,
                };
                self.styles.push((attr.name.clone(), string_value));
            }
            BindingKind::Property | BindingKind::TwoWayProperty => {
                self.bindings.push(attr.name.clone());
            }
            BindingKind::Template => {
                self.template.push(attr.name.clone());
            }
            BindingKind::I18n => {
                self.i18n.push(attr.name.clone());
            }
            BindingKind::Animation | BindingKind::LegacyAnimation => {
                // Animation bindings are handled separately
            }
        }
    }

    /// Add an extracted attribute for host binding compilation.
    ///
    /// For host bindings, Property and TwoWayProperty bindings should NOT be added
    /// to the hostAttrs array because they are handled by the hostBindings function.
    /// Only static attributes (Attribute, ClassName, StyleProperty) should be included.
    ///
    /// This matches Angular's behavior where hostAttrs only contains static attributes,
    /// not dynamic property/event bindings which are emitted in hostBindings instead.
    fn add_for_host(&mut self, attr: &ExtractedAttributeOp<'a>) {
        // Same duplicate handling logic as add()
        let allow_duplicates = self.compatibility == CompatibilityMode::TemplateDefinitionBuilder
            && matches!(
                attr.binding_kind,
                BindingKind::Attribute | BindingKind::ClassName | BindingKind::StyleProperty
            );
        if !allow_duplicates && self.is_known(attr.binding_kind, attr.name.as_str()) {
            return;
        }

        match attr.binding_kind {
            BindingKind::Attribute => {
                // For attributes, we store namespace, name, and value
                let value = Self::extract_attribute_value(attr);

                // Check for ngProjectAs attribute - store its value for special handling
                // ngProjectAs must have a string literal value, not an i18n variable reference
                if attr.name.as_str() == "ngProjectAs" {
                    if let Some(AttributeValue::String(ref val)) = value {
                        self.project_as = Some(val.clone());
                    }
                }

                self.attributes.push((attr.namespace.clone(), attr.name.clone(), value));
            }
            BindingKind::ClassName => {
                self.classes.push(attr.name.clone());
            }
            BindingKind::StyleProperty => {
                // Styles only support string values, not i18n variable references
                let value = Self::extract_attribute_value(attr);
                let string_value = match value {
                    Some(AttributeValue::String(s)) => Some(s),
                    _ => None,
                };
                self.styles.push((attr.name.clone(), string_value));
            }
            // For host bindings, skip Property and TwoWayProperty bindings.
            // These are dynamic bindings that belong in hostBindings function,
            // not in the static hostAttrs array.
            BindingKind::Property | BindingKind::TwoWayProperty => {
                // Skip - these are handled by hostBindings, not hostAttrs
            }
            BindingKind::Template => {
                self.template.push(attr.name.clone());
            }
            BindingKind::I18n => {
                self.i18n.push(attr.name.clone());
            }
            BindingKind::Animation | BindingKind::LegacyAnimation => {
                // Animation bindings are handled separately
            }
        }
    }

    /// Extracts the attribute value from an ExtractedAttributeOp.
    ///
    /// For static attributes, the value is stored as a literal string expression.
    /// For i18n attributes, the value may be a ReadVar expression referencing an i18n variable.
    /// For truthy expressions (e.g., boolean attributes), we return an empty string.
    fn extract_attribute_value(attr: &ExtractedAttributeOp<'a>) -> Option<AttributeValue<'a>> {
        use crate::ast::expression::AngularExpression;
        use crate::output::ast::LiteralValue;

        // First check if we have an actual value expression
        if let Some(value_expr) = &attr.value {
            // Try to extract the value from the expression
            match value_expr.as_ref() {
                IrExpression::OutputExpr(output_expr) => {
                    match output_expr.as_ref() {
                        OutputExpression::Literal(lit) => {
                            if let LiteralValue::String(s) = &lit.value {
                                return Some(AttributeValue::String(s.clone()));
                            }
                        }
                        // Handle ReadVar expressions (i18n variable references)
                        OutputExpression::ReadVar(read_var) => {
                            return Some(AttributeValue::I18nVar(read_var.name.clone()));
                        }
                        _ => {}
                    }
                }
                // Handle IrExpression::Ast for style values created by parse_extracted_styles
                IrExpression::Ast(ast_expr) => {
                    if let AngularExpression::LiteralPrimitive(lit) = ast_expr.as_ref() {
                        if let crate::ast::expression::LiteralValue::String(s) = &lit.value {
                            return Some(AttributeValue::String(s.clone()));
                        }
                    }
                }
                _ => {}
            }
        }

        // Fall back to truthy expression check (for boolean attributes)
        if attr.truthy_expression { Some(AttributeValue::String(Atom::from(""))) } else { None }
    }

    fn is_empty(&self) -> bool {
        self.attributes.is_empty()
            && self.classes.is_empty()
            && self.styles.is_empty()
            && self.bindings.is_empty()
            && self.template.is_empty()
            && self.i18n.is_empty()
    }
}

/// Serializes element attributes into a ConstValue::Array.
fn serialize_attributes<'a>(
    allocator: &'a oxc_allocator::Allocator,
    attrs: &ElementAttributes<'a>,
) -> ConstValue<'a> {
    let mut elements = OxcVec::new_in(allocator);

    // Add static attributes
    for (namespace, name, value) in &attrs.attributes {
        if let Some(ns) = namespace {
            // Namespaced attribute: [NamespaceUri, namespace, name, value?]
            elements.push(ConstValue::Number(AttributeMarker::NamespaceUri as i32 as f64));
            elements.push(ConstValue::String(ns.clone()));
        }
        elements.push(ConstValue::String(name.clone()));
        if let Some(val) = value {
            match val {
                AttributeValue::String(s) => {
                    elements.push(ConstValue::String(s.clone()));
                }
                AttributeValue::I18nVar(var_name) => {
                    // For i18n variable references, create a ReadVar expression.
                    // This ensures different i18n variables (I18N_0, I18N_1, etc.) are
                    // NOT deduplicated because ConstValue::Expression uses is_equivalent
                    // which compares ReadVar names.
                    let read_var = OutputExpression::ReadVar(Box::new_in(
                        crate::output::ast::ReadVarExpr {
                            name: var_name.clone(),
                            source_span: None,
                        },
                        allocator,
                    ));
                    elements.push(ConstValue::Expression(read_var));
                }
            }
        }
    }

    // Add ProjectAs marker and parsed CSS selector if ngProjectAs is present
    // This must come after the static attributes and before classes/styles/bindings
    // Reference: Angular's const_collection.ts lines 251-258
    if let Some(ref project_as) = attrs.project_as {
        // Parse the ngProjectAs value as a CSS selector
        // We only take the first selector (Angular doesn't support multiple selectors in ngProjectAs)
        let r3_selectors = parse_selector_to_r3_selector(project_as.as_str());
        if let Some(first_selector) = r3_selectors.first() {
            // Add the ProjectAs marker (value 5)
            elements.push(ConstValue::Number(AttributeMarker::ProjectAs as i32 as f64));

            // Add the parsed selector as an array
            let selector_elements = r3_selector_to_output_expr(allocator, first_selector);
            let selector_array = OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: selector_elements, source_span: None },
                allocator,
            ));
            elements.push(ConstValue::Expression(selector_array));
        }
    }

    // Add classes marker and class names
    if !attrs.classes.is_empty() {
        elements.push(ConstValue::Number(AttributeMarker::Classes as i32 as f64));
        for class in &attrs.classes {
            elements.push(ConstValue::String(class.clone()));
        }
    }

    // Add styles marker and style properties
    if !attrs.styles.is_empty() {
        elements.push(ConstValue::Number(AttributeMarker::Styles as i32 as f64));
        for (name, value) in &attrs.styles {
            elements.push(ConstValue::String(name.clone()));
            if let Some(val) = value {
                elements.push(ConstValue::String(val.clone()));
            }
        }
    }

    // Add bindings marker and binding names
    if !attrs.bindings.is_empty() {
        elements.push(ConstValue::Number(AttributeMarker::Bindings as i32 as f64));
        for name in &attrs.bindings {
            elements.push(ConstValue::String(name.clone()));
        }
    }

    // Add template marker and template bindings
    if !attrs.template.is_empty() {
        elements.push(ConstValue::Number(AttributeMarker::Template as i32 as f64));
        for name in &attrs.template {
            elements.push(ConstValue::String(name.clone()));
        }
    }

    // Add i18n marker and i18n attributes
    if !attrs.i18n.is_empty() {
        elements.push(ConstValue::Number(AttributeMarker::I18n as i32 as f64));
        for name in &attrs.i18n {
            elements.push(ConstValue::String(name.clone()));
        }
    }

    ConstValue::Array(elements)
}

/// Serializes element attributes into an OutputExpression::LiteralArray.
///
/// This is used for Projection ops (ng-content) where the attributes array
/// is passed directly to the instruction, unlike element ops which use a const index.
fn serialize_attributes_to_array_expr<'a>(
    allocator: &'a oxc_allocator::Allocator,
    attrs: &ElementAttributes<'a>,
) -> OutputExpression<'a> {
    let mut entries = OxcVec::new_in(allocator);

    // Add static attributes
    for (namespace, name, value) in &attrs.attributes {
        if let Some(ns) = namespace {
            // Namespaced attribute: [NamespaceUri, namespace, name, value?]
            entries.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(AttributeMarker::NamespaceUri as i32 as f64),
                    source_span: None,
                },
                allocator,
            )));
            entries.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(ns.clone()), source_span: None },
                allocator,
            )));
        }
        entries.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
            allocator,
        )));
        if let Some(val) = value {
            match val {
                AttributeValue::String(s) => {
                    entries.push(OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::String(s.clone()), source_span: None },
                        allocator,
                    )));
                }
                AttributeValue::I18nVar(var_name) => {
                    // For i18n variable references, create a ReadVar expression
                    entries.push(OutputExpression::ReadVar(Box::new_in(
                        crate::output::ast::ReadVarExpr {
                            name: var_name.clone(),
                            source_span: None,
                        },
                        allocator,
                    )));
                }
            }
        }
    }

    // Add ProjectAs marker and parsed CSS selector if ngProjectAs is present
    // This must come after the static attributes and before classes/styles/bindings
    // Reference: Angular's const_collection.ts lines 251-258
    if let Some(ref project_as) = attrs.project_as {
        // Parse the ngProjectAs value as a CSS selector
        // We only take the first selector (Angular doesn't support multiple selectors in ngProjectAs)
        let r3_selectors = parse_selector_to_r3_selector(project_as.as_str());
        if let Some(first_selector) = r3_selectors.first() {
            // Add the ProjectAs marker (value 5)
            entries.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(AttributeMarker::ProjectAs as i32 as f64),
                    source_span: None,
                },
                allocator,
            )));

            // Add the parsed selector as an array
            let selector_elements = r3_selector_to_output_expr(allocator, first_selector);
            entries.push(OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: selector_elements, source_span: None },
                allocator,
            )));
        }
    }

    // Add classes marker and class names
    if !attrs.classes.is_empty() {
        entries.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr {
                value: LiteralValue::Number(AttributeMarker::Classes as i32 as f64),
                source_span: None,
            },
            allocator,
        )));
        for class in &attrs.classes {
            entries.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(class.clone()), source_span: None },
                allocator,
            )));
        }
    }

    // Add styles marker and style properties
    if !attrs.styles.is_empty() {
        entries.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr {
                value: LiteralValue::Number(AttributeMarker::Styles as i32 as f64),
                source_span: None,
            },
            allocator,
        )));
        for (name, value) in &attrs.styles {
            entries.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
                allocator,
            )));
            if let Some(val) = value {
                entries.push(OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::String(val.clone()), source_span: None },
                    allocator,
                )));
            }
        }
    }

    // Add bindings marker and binding names
    if !attrs.bindings.is_empty() {
        entries.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr {
                value: LiteralValue::Number(AttributeMarker::Bindings as i32 as f64),
                source_span: None,
            },
            allocator,
        )));
        for name in &attrs.bindings {
            entries.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
                allocator,
            )));
        }
    }

    // Add template marker and template bindings
    if !attrs.template.is_empty() {
        entries.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr {
                value: LiteralValue::Number(AttributeMarker::Template as i32 as f64),
                source_span: None,
            },
            allocator,
        )));
        for name in &attrs.template {
            entries.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
                allocator,
            )));
        }
    }

    // Add i18n marker and i18n attributes
    if !attrs.i18n.is_empty() {
        entries.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr {
                value: LiteralValue::Number(AttributeMarker::I18n as i32 as f64),
                source_span: None,
            },
            allocator,
        )));
        for name in &attrs.i18n {
            entries.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
                allocator,
            )));
        }
    }

    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries, source_span: None },
        allocator,
    ))
}

/// Represents an xref that needs a const index assigned.
/// Used to capture iteration order for const index assignment.
enum XrefToAssign {
    /// Projection xref (serialized directly, not via const pool)
    Projection(XrefId),
    /// Regular element/container xref
    Regular(XrefId),
    /// Repeater body view xref
    RepeaterBody(XrefId),
    /// Repeater empty view xref
    RepeaterEmpty(XrefId),
}

/// Collects element constants into the template consts array.
///
/// This is a direct port of Angular's `collectElementConsts` function from const_collection.ts.
///
/// Angular's approach (lines 24-79):
/// 1. Collect all ExtractedAttribute ops into `allElementAttributes` map and remove them
/// 2. For ComponentCompilationJob: iterate units and ops, calling `getConstIndex()` IMMEDIATELY
///    for each element/container op encountered
///
/// The key insight is that `getConstIndex()` must be called in iteration order to ensure
/// const indices are assigned in the exact order elements are encountered. Due to Rust's
/// borrow checker, we split this into three sub-passes:
/// - Pass 2a: Iterate ops and collect xrefs in exact iteration order
/// - Pass 2b: Call `add_const` for each xref in that collected order
/// - Pass 2c: Apply the collected indices back to ops
///
/// This preserves Angular's iteration-order semantics while satisfying the borrow checker.
pub fn collect_element_consts(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;
    let compatibility = job.compatibility_mode;

    // Collect all extracted attributes grouped by target element
    // Ported from const_collection.ts lines 26-37
    let mut all_element_attrs: FxHashMap<XrefId, ElementAttributes<'_>> = FxHashMap::default();

    // Collect view xrefs to process
    // Match TypeScript's iteration order: root first (added to views map in constructor),
    // then embedded views in insertion order.
    let view_xrefs: std::vec::Vec<_> = job.all_views().map(|v| v.xref).collect();

    // First pass: Collect extracted attributes from all views and remove them
    for view_xref in &view_xrefs {
        if let Some(view) = job.view_mut(*view_xref) {
            let mut cursor = view.create.cursor_front();
            loop {
                let should_remove = if let Some(op) = cursor.current() {
                    if let CreateOp::ExtractedAttribute(attr) = op {
                        let attrs = all_element_attrs
                            .entry(attr.target)
                            .or_insert_with(|| ElementAttributes::new(compatibility));
                        attrs.add(attr);
                        true
                    } else {
                        false
                    }
                } else {
                    break;
                };

                if should_remove {
                    cursor.remove_current();
                } else if !cursor.move_next() {
                    break;
                }
            }
        }
    }

    // Second pass (2a): Iterate ops in exact order and collect xrefs
    // This captures the iteration order that Angular's getConstIndex would be called in.
    //
    // Ported from const_collection.ts lines 40-64 iteration pattern
    let mut xrefs_to_assign: std::vec::Vec<XrefToAssign> = std::vec::Vec::new();
    let mut seen_xrefs: std::collections::HashSet<XrefId> = std::collections::HashSet::new();

    for view_xref in &view_xrefs {
        if let Some(view) = job.view(*view_xref) {
            for op in view.create.iter() {
                match op {
                    CreateOp::Projection(proj) => {
                        // Projections serialize directly (not via const pool)
                        if all_element_attrs.get(&proj.xref).is_some_and(|a| !a.is_empty()) {
                            xrefs_to_assign.push(XrefToAssign::Projection(proj.xref));
                        }
                    }
                    CreateOp::ElementStart(elem) => {
                        if !seen_xrefs.contains(&elem.xref)
                            && all_element_attrs.get(&elem.xref).is_some_and(|a| !a.is_empty())
                        {
                            seen_xrefs.insert(elem.xref);
                            xrefs_to_assign.push(XrefToAssign::Regular(elem.xref));
                        }
                    }
                    CreateOp::Element(elem) => {
                        if !seen_xrefs.contains(&elem.xref)
                            && all_element_attrs.get(&elem.xref).is_some_and(|a| !a.is_empty())
                        {
                            seen_xrefs.insert(elem.xref);
                            xrefs_to_assign.push(XrefToAssign::Regular(elem.xref));
                        }
                    }
                    CreateOp::ContainerStart(container) => {
                        if !seen_xrefs.contains(&container.xref)
                            && all_element_attrs.get(&container.xref).is_some_and(|a| !a.is_empty())
                        {
                            seen_xrefs.insert(container.xref);
                            xrefs_to_assign.push(XrefToAssign::Regular(container.xref));
                        }
                    }
                    CreateOp::Container(container) => {
                        if !seen_xrefs.contains(&container.xref)
                            && all_element_attrs.get(&container.xref).is_some_and(|a| !a.is_empty())
                        {
                            seen_xrefs.insert(container.xref);
                            xrefs_to_assign.push(XrefToAssign::Regular(container.xref));
                        }
                    }
                    CreateOp::Template(tmpl) => {
                        if !seen_xrefs.contains(&tmpl.xref)
                            && all_element_attrs.get(&tmpl.xref).is_some_and(|a| !a.is_empty())
                        {
                            seen_xrefs.insert(tmpl.xref);
                            xrefs_to_assign.push(XrefToAssign::Regular(tmpl.xref));
                        }
                    }
                    CreateOp::Conditional(cond) => {
                        if !seen_xrefs.contains(&cond.xref)
                            && all_element_attrs.get(&cond.xref).is_some_and(|a| !a.is_empty())
                        {
                            seen_xrefs.insert(cond.xref);
                            xrefs_to_assign.push(XrefToAssign::Regular(cond.xref));
                        }
                    }
                    CreateOp::ConditionalBranch(branch) => {
                        if !seen_xrefs.contains(&branch.xref)
                            && all_element_attrs.get(&branch.xref).is_some_and(|a| !a.is_empty())
                        {
                            seen_xrefs.insert(branch.xref);
                            xrefs_to_assign.push(XrefToAssign::Regular(branch.xref));
                        }
                    }
                    CreateOp::RepeaterCreate(rep) => {
                        // For repeaters: body_view first, then empty_view
                        // Ported from const_collection.ts lines 52-61
                        if !seen_xrefs.contains(&rep.body_view)
                            && all_element_attrs.get(&rep.body_view).is_some_and(|a| !a.is_empty())
                        {
                            seen_xrefs.insert(rep.body_view);
                            xrefs_to_assign.push(XrefToAssign::RepeaterBody(rep.body_view));
                        }
                        if let Some(empty_view) = rep.empty_view {
                            if !seen_xrefs.contains(&empty_view)
                                && all_element_attrs.get(&empty_view).is_some_and(|a| !a.is_empty())
                            {
                                seen_xrefs.insert(empty_view);
                                xrefs_to_assign.push(XrefToAssign::RepeaterEmpty(empty_view));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Second pass (2b): Assign const indices in collected order
    // This is where we actually call add_const, matching Angular's getConstIndex call order
    let mut element_const_indices: FxHashMap<XrefId, u32> = FxHashMap::default();
    let mut projection_attrs: FxHashMap<XrefId, OutputExpression<'_>> = FxHashMap::default();

    for xref_item in &xrefs_to_assign {
        match xref_item {
            XrefToAssign::Projection(xref) => {
                if let Some(attrs) = all_element_attrs.get(xref) {
                    if !attrs.is_empty() {
                        let attr_array = serialize_attributes_to_array_expr(allocator, attrs);
                        projection_attrs.insert(*xref, attr_array);
                    }
                }
            }
            XrefToAssign::Regular(xref)
            | XrefToAssign::RepeaterBody(xref)
            | XrefToAssign::RepeaterEmpty(xref) => {
                if !element_const_indices.contains_key(xref) {
                    if let Some(attrs) = all_element_attrs.get(xref) {
                        if !attrs.is_empty() {
                            let const_value = serialize_attributes(allocator, attrs);
                            let const_idx = job.add_const(const_value);
                            element_const_indices.insert(*xref, const_idx);
                        }
                    }
                }
            }
        }
    }

    // Second pass (2c): Apply const indices to element ops
    for view_xref in &view_xrefs {
        if let Some(view) = job.view_mut(*view_xref) {
            for op in view.create.iter_mut() {
                match op {
                    CreateOp::Projection(proj) => {
                        if let Some(attrs) = projection_attrs.remove(&proj.xref) {
                            proj.attributes = Some(attrs);
                        }
                    }
                    CreateOp::ElementStart(elem) => {
                        if let Some(&idx) = element_const_indices.get(&elem.xref) {
                            elem.attributes = Some(idx);
                        }
                    }
                    CreateOp::Element(elem) => {
                        if let Some(&idx) = element_const_indices.get(&elem.xref) {
                            elem.attributes = Some(idx);
                        }
                    }
                    CreateOp::ContainerStart(container) => {
                        if let Some(&idx) = element_const_indices.get(&container.xref) {
                            container.attributes = Some(idx);
                        }
                    }
                    CreateOp::Container(container) => {
                        if let Some(&idx) = element_const_indices.get(&container.xref) {
                            container.attributes = Some(idx);
                        }
                    }
                    CreateOp::Template(tmpl) => {
                        if let Some(&idx) = element_const_indices.get(&tmpl.xref) {
                            tmpl.attributes = Some(idx);
                        }
                    }
                    CreateOp::Conditional(cond) => {
                        if let Some(&idx) = element_const_indices.get(&cond.xref) {
                            cond.attributes = Some(idx);
                        }
                    }
                    CreateOp::ConditionalBranch(branch) => {
                        if let Some(&idx) = element_const_indices.get(&branch.xref) {
                            branch.attributes = Some(idx);
                        }
                    }
                    CreateOp::RepeaterCreate(rep) => {
                        if let Some(&idx) = element_const_indices.get(&rep.body_view) {
                            rep.attributes = Some(idx);
                        }
                        if let Some(empty_view) = rep.empty_view {
                            if let Some(&idx) = element_const_indices.get(&empty_view) {
                                rep.empty_attributes = Some(idx);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Collects element consts for host binding compilation.
///
/// Host version - only processes the root unit (no embedded views).
/// This collects all ExtractedAttribute ops from the host binding unit and
/// serializes them into `job.root.attributes` for emission as `hostAttrs`.
///
/// Ported from Angular's const_collection.ts lines 65-79:
/// ```typescript
/// } else if (job instanceof HostBindingCompilationJob) {
///   for (const [xref, attributes] of allElementAttributes.entries()) {
///     if (xref !== job.root.xref) {
///       throw new Error(...);
///     }
///     const attrArray = serializeAttributes(attributes);
///     if (attrArray.entries.length > 0) {
///       job.root.attributes = attrArray;
///     }
///   }
/// }
/// ```
pub fn collect_element_consts_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;
    let root_xref = job.root.xref;
    let compatibility = job.compatibility_mode;

    // Collect all extracted attributes from the root unit
    let mut attrs = ElementAttributes::new(compatibility);

    // First pass: Collect extracted attributes and mark them for removal
    // Use add_for_host which skips Property/TwoWayProperty bindings since those
    // are dynamic bindings that belong in hostBindings function, not hostAttrs.
    let mut cursor = job.root.create.cursor_front();
    loop {
        let should_remove = if let Some(op) = cursor.current() {
            if let CreateOp::ExtractedAttribute(attr) = op {
                // For host bindings, all attributes should target the root xref
                // Per Angular's const_collection.ts line 69-72:
                // if (xref !== job.root.xref) {
                //   throw new Error("An attribute would be const collected...");
                // }
                if attr.target != root_xref {
                    job.diagnostics.push(OxcDiagnostic::error(
                        "Host binding attribute should target root xref",
                    ));
                }
                attrs.add_for_host(attr);
                true
            } else {
                false
            }
        } else {
            break;
        };

        if should_remove {
            cursor.remove_current();
        } else if !cursor.move_next() {
            break;
        }
    }

    // Serialize the attributes and set them on the root unit
    if !attrs.is_empty() {
        let attr_array = serialize_attributes_to_array_expr(allocator, &attrs);
        job.root.attributes = Some(attr_array);
    }
}
