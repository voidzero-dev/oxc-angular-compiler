//! Angular Partial Declaration Linker.
//!
//! Processes pre-compiled Angular library code from node_modules that contains
//! partial compilation declarations (`ɵɵngDeclare*`). These declarations need
//! to be "linked" (converted to full `ɵɵdefine*` calls) at build time.
//!
//! Without this linker, Angular falls back to JIT compilation which requires
//! `@angular/compiler` at runtime.
//!
//! ## Supported Declarations
//!
//! | Partial Declaration | Linked Output |
//! |--------------------|-|
//! | `ɵɵngDeclareFactory` | Factory function |
//! | `ɵɵngDeclareInjectable` | `ɵɵdefineInjectable(...)` |
//! | `ɵɵngDeclareInjector` | `ɵɵdefineInjector(...)` |
//! | `ɵɵngDeclareNgModule` | `ɵɵdefineNgModule(...)` |
//! | `ɵɵngDeclarePipe` | `ɵɵdefinePipe(...)` |
//! | `ɵɵngDeclareDirective` | `ɵɵdefineDirective(...)` |
//! | `ɵɵngDeclareComponent` | `ɵɵdefineComponent(...)` |
//! | `ɵɵngDeclareClassMetadata` | `ɵɵsetClassMetadata(...)` |
//!
//! ## Usage
//!
//! ```ignore
//! use oxc_allocator::Allocator;
//! use oxc_angular_compiler::linker::link;
//!
//! let allocator = Allocator::default();
//! let code = "static ɵfac = i0.ɵɵngDeclareFactory({...});";
//! let result = link(&allocator, code, "common.mjs");
//! println!("{}", result.code);
//! ```

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, CallExpression, Expression, ObjectExpression, ObjectPropertyKind, Program,
    PropertyKey, Statement,
};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

use crate::optimizer::Edit;

/// Partial declaration function names to link.
const DECLARE_FACTORY: &str = "\u{0275}\u{0275}ngDeclareFactory";
const DECLARE_INJECTABLE: &str = "\u{0275}\u{0275}ngDeclareInjectable";
const DECLARE_INJECTOR: &str = "\u{0275}\u{0275}ngDeclareInjector";
const DECLARE_NG_MODULE: &str = "\u{0275}\u{0275}ngDeclareNgModule";
const DECLARE_PIPE: &str = "\u{0275}\u{0275}ngDeclarePipe";
const DECLARE_DIRECTIVE: &str = "\u{0275}\u{0275}ngDeclareDirective";
const DECLARE_COMPONENT: &str = "\u{0275}\u{0275}ngDeclareComponent";
const DECLARE_CLASS_METADATA: &str = "\u{0275}\u{0275}ngDeclareClassMetadata";
const DECLARE_CLASS_METADATA_ASYNC: &str = "\u{0275}\u{0275}ngDeclareClassMetadataAsync";

/// Result of linking an Angular package file.
#[derive(Debug, Clone, Default)]
pub struct LinkResult {
    /// The linked code.
    pub code: String,
    /// Source map (if enabled).
    pub map: Option<String>,
    /// Whether any declarations were linked.
    pub linked: bool,
}

/// Link Angular partial declarations in a JavaScript file.
///
/// Scans the code for `ɵɵngDeclare*` calls and replaces them with their
/// fully compiled equivalents.
pub fn link(allocator: &Allocator, code: &str, filename: &str) -> LinkResult {
    // Quick check: if no declarations, return early
    if !code.contains("\u{0275}\u{0275}ngDeclare") {
        return LinkResult { code: code.to_string(), map: None, linked: false };
    }

    let source_type = SourceType::from_path(filename).unwrap_or(SourceType::mjs());
    let parser_result = Parser::new(allocator, code, source_type).parse();

    if parser_result.panicked || !parser_result.errors.is_empty() {
        return LinkResult { code: code.to_string(), map: None, linked: false };
    }

    let program = parser_result.program;
    let mut edits: Vec<Edit> = Vec::new();

    // Walk all statements looking for ɵɵngDeclare* calls
    collect_declaration_edits(&program, code, &mut edits);

    if edits.is_empty() {
        return LinkResult { code: code.to_string(), map: None, linked: false };
    }

    let linked_code = crate::optimizer::apply_edits(code, edits);

    LinkResult { code: linked_code, map: None, linked: true }
}

/// Recursively walk the AST to find all ɵɵngDeclare* calls and generate edits.
fn collect_declaration_edits(program: &Program<'_>, source: &str, edits: &mut Vec<Edit>) {
    for stmt in &program.body {
        walk_statement(stmt, source, edits);
    }
}

/// Walk a statement looking for ɵɵngDeclare* calls.
fn walk_statement(stmt: &Statement<'_>, source: &str, edits: &mut Vec<Edit>) {
    match stmt {
        Statement::ExpressionStatement(expr_stmt) => {
            walk_expression(&expr_stmt.expression, source, edits);
        }
        Statement::ClassDeclaration(class_decl) => {
            walk_class_body(&class_decl.body, source, edits);
        }
        Statement::VariableDeclaration(var_decl) => {
            for decl in &var_decl.declarations {
                if let Some(init) = &decl.init {
                    walk_expression(init, source, edits);
                }
            }
        }
        Statement::ReturnStatement(ret) => {
            if let Some(ref arg) = ret.argument {
                walk_expression(arg, source, edits);
            }
        }
        Statement::BlockStatement(block) => {
            for stmt in &block.body {
                walk_statement(stmt, source, edits);
            }
        }
        Statement::IfStatement(if_stmt) => {
            walk_statement(&if_stmt.consequent, source, edits);
            if let Some(ref alt) = if_stmt.alternate {
                walk_statement(alt, source, edits);
            }
        }
        Statement::ForStatement(for_stmt) => {
            walk_statement(&for_stmt.body, source, edits);
        }
        Statement::ForInStatement(for_in) => {
            walk_statement(&for_in.body, source, edits);
        }
        Statement::ForOfStatement(for_of) => {
            walk_statement(&for_of.body, source, edits);
        }
        Statement::WhileStatement(while_stmt) => {
            walk_statement(&while_stmt.body, source, edits);
        }
        Statement::DoWhileStatement(do_while) => {
            walk_statement(&do_while.body, source, edits);
        }
        Statement::TryStatement(try_stmt) => {
            for stmt in &try_stmt.block.body {
                walk_statement(stmt, source, edits);
            }
            if let Some(ref handler) = try_stmt.handler {
                for stmt in &handler.body.body {
                    walk_statement(stmt, source, edits);
                }
            }
            if let Some(ref finalizer) = try_stmt.finalizer {
                for stmt in &finalizer.body {
                    walk_statement(stmt, source, edits);
                }
            }
        }
        Statement::SwitchStatement(switch_stmt) => {
            for case in &switch_stmt.cases {
                for stmt in &case.consequent {
                    walk_statement(stmt, source, edits);
                }
            }
        }
        Statement::LabeledStatement(labeled) => {
            walk_statement(&labeled.body, source, edits);
        }
        Statement::FunctionDeclaration(func_decl) => {
            if let Some(ref body) = func_decl.body {
                for stmt in &body.statements {
                    walk_statement(stmt, source, edits);
                }
            }
        }
        Statement::ExportNamedDeclaration(export_decl) => {
            if let Some(ref decl) = export_decl.declaration {
                walk_declaration(decl, source, edits);
            }
        }
        Statement::ExportDefaultDeclaration(export_default) => match &export_default.declaration {
            oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class_decl) => {
                walk_class_body(&class_decl.body, source, edits);
            }
            oxc_ast::ast::ExportDefaultDeclarationKind::FunctionDeclaration(func_decl) => {
                if let Some(ref body) = func_decl.body {
                    for stmt in &body.statements {
                        walk_statement(stmt, source, edits);
                    }
                }
            }
            _ => {
                if let Some(expr) = export_default.declaration.as_expression() {
                    walk_expression(expr, source, edits);
                }
            }
        },
        _ => {}
    }
}

/// Walk a class body looking for ɵɵngDeclare* calls in property definitions and static blocks.
fn walk_class_body(body: &oxc_ast::ast::ClassBody<'_>, source: &str, edits: &mut Vec<Edit>) {
    for element in &body.body {
        if let oxc_ast::ast::ClassElement::PropertyDefinition(prop) = element {
            if let Some(ref value) = prop.value {
                walk_expression(value, source, edits);
            }
        }
        if let oxc_ast::ast::ClassElement::StaticBlock(block) = element {
            for stmt in &block.body {
                walk_statement(stmt, source, edits);
            }
        }
        if let oxc_ast::ast::ClassElement::MethodDefinition(method) = element {
            if let Some(ref body) = method.value.body {
                for stmt in &body.statements {
                    walk_statement(stmt, source, edits);
                }
            }
        }
    }
}

/// Walk a declaration (from export statements) looking for ɵɵngDeclare* calls.
fn walk_declaration(decl: &oxc_ast::ast::Declaration<'_>, source: &str, edits: &mut Vec<Edit>) {
    match decl {
        oxc_ast::ast::Declaration::VariableDeclaration(var_decl) => {
            for d in &var_decl.declarations {
                if let Some(init) = &d.init {
                    walk_expression(init, source, edits);
                }
            }
        }
        oxc_ast::ast::Declaration::FunctionDeclaration(func_decl) => {
            if let Some(ref body) = func_decl.body {
                for stmt in &body.statements {
                    walk_statement(stmt, source, edits);
                }
            }
        }
        oxc_ast::ast::Declaration::ClassDeclaration(class_decl) => {
            walk_class_body(&class_decl.body, source, edits);
        }
        _ => {}
    }
}

/// Walk an expression looking for ɵɵngDeclare* calls.
fn walk_expression(expr: &Expression<'_>, source: &str, edits: &mut Vec<Edit>) {
    match expr {
        Expression::CallExpression(call) => {
            if let Some(name) = get_declare_name(call) {
                if let Some(edit) = link_declaration(name, call, source) {
                    edits.push(edit);
                    return;
                }
            }
            // Walk arguments recursively
            for arg in &call.arguments {
                if let Argument::SpreadElement(_) = arg {
                    continue;
                }
                walk_expression(arg.to_expression(), source, edits);
            }
        }
        Expression::AssignmentExpression(assign) => {
            walk_expression(&assign.right, source, edits);
        }
        Expression::SequenceExpression(seq) => {
            for expr in &seq.expressions {
                walk_expression(expr, source, edits);
            }
        }
        Expression::ConditionalExpression(cond) => {
            walk_expression(&cond.consequent, source, edits);
            walk_expression(&cond.alternate, source, edits);
        }
        Expression::LogicalExpression(logical) => {
            walk_expression(&logical.left, source, edits);
            walk_expression(&logical.right, source, edits);
        }
        Expression::ParenthesizedExpression(paren) => {
            walk_expression(&paren.expression, source, edits);
        }
        Expression::ArrowFunctionExpression(arrow) => {
            for stmt in &arrow.body.statements {
                walk_statement(stmt, source, edits);
            }
        }
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                for stmt in &body.statements {
                    walk_statement(stmt, source, edits);
                }
            }
        }
        Expression::ClassExpression(class_expr) => {
            walk_class_body(&class_expr.body, source, edits);
        }
        _ => {}
    }
}

/// Check if a call expression is a ɵɵngDeclare* call and return the declaration name.
fn get_declare_name<'a>(call: &'a CallExpression<'a>) -> Option<&'a str> {
    let name = match &call.callee {
        Expression::Identifier(ident) => ident.name.as_str(),
        Expression::StaticMemberExpression(member) => member.property.name.as_str(),
        _ => return None,
    };

    match name {
        DECLARE_FACTORY
        | DECLARE_INJECTABLE
        | DECLARE_INJECTOR
        | DECLARE_NG_MODULE
        | DECLARE_PIPE
        | DECLARE_DIRECTIVE
        | DECLARE_COMPONENT
        | DECLARE_CLASS_METADATA
        | DECLARE_CLASS_METADATA_ASYNC => Some(name),
        _ => None,
    }
}

/// Get the Angular import namespace (e.g., "i0") from the callee.
fn get_ng_import_namespace<'a>(call: &'a CallExpression<'a>) -> &'a str {
    match &call.callee {
        Expression::StaticMemberExpression(member) => {
            if let Expression::Identifier(ident) = &member.object {
                return ident.name.as_str();
            }
            "i0"
        }
        _ => "i0",
    }
}

/// Get the metadata object from a ɵɵngDeclare* call's first argument.
fn get_metadata_object<'a>(call: &'a CallExpression<'a>) -> Option<&'a ObjectExpression<'a>> {
    call.arguments.first().and_then(|arg| {
        if let Argument::ObjectExpression(obj) = arg { Some(obj.as_ref()) } else { None }
    })
}

/// Extract a string property value from an object expression.
fn get_string_property<'a>(obj: &'a ObjectExpression<'a>, name: &str) -> Option<&'a str> {
    for prop in &obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if matches!(&prop.key, PropertyKey::StaticIdentifier(ident) if ident.name == name) {
                if let Expression::StringLiteral(lit) = &prop.value {
                    return Some(lit.value.as_str());
                }
            }
        }
    }
    None
}

/// Extract the source text of a property value from an object expression.
fn get_property_source<'a>(
    obj: &'a ObjectExpression<'a>,
    name: &str,
    source: &'a str,
) -> Option<&'a str> {
    for prop in &obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if matches!(&prop.key, PropertyKey::StaticIdentifier(ident) if ident.name == name) {
                let span = prop.value.span();
                return Some(&source[span.start as usize..span.end as usize]);
            }
        }
    }
    None
}

/// Check if a property exists in an object expression.
fn has_property(obj: &ObjectExpression<'_>, name: &str) -> bool {
    obj.properties.iter().any(|prop| {
        matches!(prop,
            ObjectPropertyKind::ObjectProperty(p)
            if matches!(&p.key, PropertyKey::StaticIdentifier(ident) if ident.name == name)
        )
    })
}

/// Extract an object expression property value from an object expression.
fn get_object_property<'a>(
    obj: &'a ObjectExpression<'a>,
    name: &str,
) -> Option<&'a ObjectExpression<'a>> {
    for prop in &obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if matches!(&prop.key, PropertyKey::StaticIdentifier(ident) if ident.name == name) {
                if let Expression::ObjectExpression(inner) = &prop.value {
                    return Some(inner.as_ref());
                }
            }
        }
    }
    None
}

/// Extract boolean property value.
fn get_bool_property(obj: &ObjectExpression<'_>, name: &str) -> Option<bool> {
    for prop in &obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if matches!(&prop.key, PropertyKey::StaticIdentifier(ident) if ident.name == name) {
                if let Expression::BooleanLiteral(lit) = &prop.value {
                    return Some(lit.value);
                }
            }
        }
    }
    None
}

/// Extract the `deps` array from a factory metadata object and generate inject calls.
fn extract_deps_source(obj: &ObjectExpression<'_>, source: &str, ns: &str) -> String {
    for prop in &obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if matches!(&prop.key, PropertyKey::StaticIdentifier(ident) if ident.name == "deps") {
                if let Expression::ArrayExpression(arr) = &prop.value {
                    if arr.elements.is_empty() {
                        return String::new();
                    }
                    // Generate inject calls for each dependency
                    let deps: Vec<String> = arr
                        .elements
                        .iter()
                        .filter_map(|el| {
                            use oxc_ast::ast::ArrayExpressionElement;
                            let expr = match el {
                                ArrayExpressionElement::SpreadElement(_) => return None,
                                _ => el.to_expression(),
                            };
                            let span = expr.span();
                            let dep_source = &source[span.start as usize..span.end as usize];

                            // Check if it's an object with token/attribute/flags
                            if let Expression::ObjectExpression(dep_obj) = expr {
                                let token = get_property_source(dep_obj.as_ref(), "token", source);
                                let optional = get_bool_property(dep_obj.as_ref(), "optional");
                                let self_flag = get_bool_property(dep_obj.as_ref(), "self");
                                let skip_self = get_bool_property(dep_obj.as_ref(), "skipSelf");
                                let host = get_bool_property(dep_obj.as_ref(), "host");
                                let attribute =
                                    get_property_source(dep_obj.as_ref(), "attribute", source);

                                if let Some(attr) = attribute {
                                    return Some(format!(
                                        "{ns}.\u{0275}\u{0275}injectAttribute({attr})"
                                    ));
                                }

                                if let Some(token) = token {
                                    let mut flags = 0u32;
                                    if optional == Some(true) {
                                        flags |= 8;
                                    }
                                    if self_flag == Some(true) {
                                        flags |= 2;
                                    }
                                    if skip_self == Some(true) {
                                        flags |= 4;
                                    }
                                    if host == Some(true) {
                                        flags |= 1;
                                    }
                                    if flags != 0 {
                                        return Some(format!(
                                            "{ns}.\u{0275}\u{0275}inject({token}, {flags})"
                                        ));
                                    }
                                    return Some(format!("{ns}.\u{0275}\u{0275}inject({token})"));
                                }
                                Some(format!("{ns}.\u{0275}\u{0275}inject({dep_source})"))
                            } else {
                                Some(format!("{ns}.\u{0275}\u{0275}inject({dep_source})"))
                            }
                        })
                        .collect();
                    return deps.join(", ");
                }
            }
        }
    }
    String::new()
}

/// Parse a CSS selector string into Angular's internal selector array format.
///
/// Angular represents selectors as nested arrays:
/// - `"app-root"` → `[["app-root"]]`
/// - `"[ngClass]"` → `[["", "ngClass", ""]]`
/// - `"[attr=value]"` → `[["", "attr", "value"]]`
/// - `"div[ngClass]"` → `[["div", "ngClass", ""]]`
/// - `"[a],[b]"` → `[["", "a", ""], ["", "b", ""]]`
/// - `".cls"` → `[["", "class", "cls"]]`
fn parse_selector(selector: &str) -> String {
    let selectors: Vec<String> =
        selector.split(',').map(|s| parse_single_selector(s.trim())).collect();
    format!("[{}]", selectors.join(", "))
}

/// Parse a single selector (no commas) into Angular's array format.
fn parse_single_selector(selector: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut remaining = selector;

    // Extract tag name (everything before first [ or . or :)
    let tag_end = remaining
        .find(|c: char| c == '[' || c == '.' || c == ':' || c == '#')
        .unwrap_or(remaining.len());
    let tag = &remaining[..tag_end];
    remaining = &remaining[tag_end..];

    if !tag.is_empty() {
        parts.push(format!("\"{}\"", tag));
    } else {
        parts.push("\"\"".to_string());
    }

    // Extract attribute selectors [attr] or [attr=value]
    while let Some(bracket_start) = remaining.find('[') {
        let bracket_end = remaining[bracket_start..].find(']').map(|i| bracket_start + i);
        if let Some(end) = bracket_end {
            let attr_content = &remaining[bracket_start + 1..end];
            if let Some(eq_pos) = attr_content.find('=') {
                let attr_name = &attr_content[..eq_pos];
                let attr_value = attr_content[eq_pos + 1..].trim_matches('"').trim_matches('\'');
                parts.push(format!("\"{}\"", attr_name));
                parts.push(format!("\"{}\"", attr_value));
            } else {
                parts.push(format!("\"{}\"", attr_content));
                parts.push("\"\"".to_string());
            }
            remaining = &remaining[end + 1..];
        } else {
            break;
        }
    }

    // Extract class selectors .className
    let mut class_remaining = remaining;
    while let Some(dot_pos) = class_remaining.find('.') {
        let class_end = class_remaining[dot_pos + 1..]
            .find(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
            .map(|i| dot_pos + 1 + i)
            .unwrap_or(class_remaining.len());
        let class_name = &class_remaining[dot_pos + 1..class_end];
        if !class_name.is_empty() {
            parts.push("\"class\"".to_string());
            parts.push(format!("\"{}\"", class_name));
        }
        class_remaining = &class_remaining[class_end..];
    }

    format!("[{}]", parts.join(", "))
}

/// Build the `hostAttrs` flat array from the partial declaration's `host` object.
///
/// The `host` object in partial declarations has sub-properties:
/// - `attributes`: `{ "role": "tree", "tabindex": "-1" }` → `["role", "tree", "tabindex", "-1"]`
/// - `classAttribute`: `"cdk-tree"` → `[1, "cdk-tree"]` (1 = AttributeMarker.Classes)
/// - `styleAttribute`: `"display: block"` → `[2, "display: block"]` (2 = AttributeMarker.Styles)
///
/// Properties and listeners go into `hostBindings`, not `hostAttrs`.
fn build_host_attrs(host_obj: &ObjectExpression<'_>, source: &str) -> String {
    let mut attrs: Vec<String> = Vec::new();

    // Static attributes: { "role": "tree", "tabindex": "-1" }
    if let Some(attr_obj) = get_object_property(host_obj, "attributes") {
        for prop in &attr_obj.properties {
            if let ObjectPropertyKind::ObjectProperty(p) = prop {
                let key = match &p.key {
                    PropertyKey::StaticIdentifier(ident) => ident.name.to_string(),
                    PropertyKey::StringLiteral(s) => s.value.to_string(),
                    _ => continue,
                };
                let value_span = p.value.span();
                let value = &source[value_span.start as usize..value_span.end as usize];
                attrs.push(format!("\"{key}\""));
                attrs.push(value.to_string());
            }
        }
    }

    // Class attribute: "cdk-tree" → [1, "cdk-tree"]
    // AttributeMarker.Classes = 1
    if let Some(class_attr) = get_string_property(host_obj, "classAttribute") {
        // Split classes and add each separately
        attrs.push("1".to_string()); // AttributeMarker.Classes
        for class in class_attr.split_whitespace() {
            attrs.push(format!("\"{class}\""));
        }
    }

    // Style attribute: "display: block" → [2, "display", "block"]
    // AttributeMarker.Styles = 2
    if let Some(style_attr) = get_string_property(host_obj, "styleAttribute") {
        attrs.push("2".to_string()); // AttributeMarker.Styles
        // Parse style string into key-value pairs
        for declaration in style_attr.split(';') {
            let declaration = declaration.trim();
            if declaration.is_empty() {
                continue;
            }
            if let Some(colon_pos) = declaration.find(':') {
                let prop = declaration[..colon_pos].trim();
                let val = declaration[colon_pos + 1..].trim();
                attrs.push(format!("\"{prop}\""));
                attrs.push(format!("\"{val}\""));
            }
        }
    }

    attrs.join(", ")
}

/// Get the factory target from metadata.
fn get_factory_target(obj: &ObjectExpression<'_>, source: &str) -> &'static str {
    if let Some(target_src) = get_property_source(obj, "target", source) {
        if target_src.contains("Pipe") {
            return "Pipe";
        }
        if target_src.contains("Directive") || target_src.contains("Component") {
            return "Directive";
        }
        if target_src.contains("NgModule") {
            return "NgModule";
        }
    }
    "Injectable"
}

/// Link a single ɵɵngDeclare* call, generating the replacement code.
fn link_declaration(name: &str, call: &CallExpression<'_>, source: &str) -> Option<Edit> {
    let meta = get_metadata_object(call)?;
    let ns = get_ng_import_namespace(call);
    let type_name = get_property_source(meta, "type", source)?;

    let replacement = match name {
        DECLARE_FACTORY => link_factory(meta, source, ns, type_name),
        DECLARE_INJECTABLE => link_injectable(meta, source, ns, type_name),
        DECLARE_INJECTOR => link_injector(meta, source, ns, type_name),
        DECLARE_NG_MODULE => link_ng_module(meta, source, ns, type_name),
        DECLARE_PIPE => link_pipe(meta, source, ns, type_name),
        DECLARE_CLASS_METADATA => link_class_metadata(meta, source, ns, type_name),
        DECLARE_CLASS_METADATA_ASYNC => link_class_metadata_async(meta, source, ns, type_name),
        DECLARE_DIRECTIVE => link_directive(meta, source, ns, type_name),
        // Skip component linking: template compilation is not yet implemented.
        // Replacing ɵɵngDeclareComponent with an empty template would silently break
        // all library component rendering. Leave the partial declaration intact so
        // Angular's runtime can JIT-compile it via @angular/compiler.
        DECLARE_COMPONENT => return None,
        _ => return None,
    };

    let replacement = replacement?;
    Some(Edit::replace(call.span.start, call.span.end, replacement))
}

/// Link ɵɵngDeclareFactory → factory function.
fn link_factory(
    meta: &ObjectExpression<'_>,
    source: &str,
    ns: &str,
    type_name: &str,
) -> Option<String> {
    let target = get_factory_target(meta, source);

    // Check if deps are specified
    let has_deps = has_property(meta, "deps");

    if !has_deps {
        // Inherited factory (no constructor) - use getInheritedFactory
        Some(format!(
            "/*@__PURE__*/ (() => {{\n\
            let \u{0275}{type_name}_BaseFactory;\n\
            return function {type_name}_Factory(__ngFactoryType__) {{\n\
              return (\u{0275}{type_name}_BaseFactory || (\u{0275}{type_name}_BaseFactory = {ns}.\u{0275}\u{0275}getInheritedFactory({type_name})))(__ngFactoryType__ || {type_name});\n\
            }};\n\
            }})()"
        ))
    } else {
        let deps = extract_deps_source(meta, source, ns);

        if target == "Pipe" {
            // Pipes use ɵɵdirectiveInject instead of ɵɵinject
            let deps_pipe = deps.replace(
                &format!("{ns}.\u{0275}\u{0275}inject("),
                &format!("{ns}.\u{0275}\u{0275}directiveInject("),
            );
            Some(format!(
                "function {type_name}_Factory(__ngFactoryType__) {{\n\
                return new (__ngFactoryType__ || {type_name})({deps_pipe});\n\
                }}"
            ))
        } else if target == "Directive" {
            let deps_dir = deps.replace(
                &format!("{ns}.\u{0275}\u{0275}inject("),
                &format!("{ns}.\u{0275}\u{0275}directiveInject("),
            );
            Some(format!(
                "function {type_name}_Factory(__ngFactoryType__) {{\n\
                return new (__ngFactoryType__ || {type_name})({deps_dir});\n\
                }}"
            ))
        } else {
            Some(format!(
                "function {type_name}_Factory(__ngFactoryType__) {{\n\
                return new (__ngFactoryType__ || {type_name})({deps});\n\
                }}"
            ))
        }
    }
}

/// Link ɵɵngDeclareInjectable → ɵɵdefineInjectable.
///
/// For `useClass` and `useFactory` with deps, we generate a wrapper factory that calls
/// `ɵɵinject()` inside the factory body (deferred), not in a `deps` array (eager).
/// Eager inject calls would fail with NG0203 during static class initialization.
fn link_injectable(
    meta: &ObjectExpression<'_>,
    source: &str,
    ns: &str,
    type_name: &str,
) -> Option<String> {
    let provided_in = get_property_source(meta, "providedIn", source).unwrap_or("null");

    // Check for useClass, useFactory, useExisting, useValue
    if let Some(use_class) = get_property_source(meta, "useClass", source) {
        if has_property(meta, "deps") {
            let deps = extract_deps_source(meta, source, ns);
            return Some(format!(
                "{ns}.\u{0275}\u{0275}defineInjectable({{ token: {type_name}, factory: function {type_name}_Factory() {{ return new ({use_class})({deps}); }}, providedIn: {provided_in} }})"
            ));
        }
        return Some(format!(
            "{ns}.\u{0275}\u{0275}defineInjectable({{ token: {type_name}, factory: function {type_name}_Factory() {{ return new ({use_class})(); }}, providedIn: {provided_in} }})"
        ));
    }

    if let Some(use_factory) = get_property_source(meta, "useFactory", source) {
        if has_property(meta, "deps") {
            let deps = extract_deps_source(meta, source, ns);
            // Wrap the user factory: call inject() inside the wrapper, pass results as args
            return Some(format!(
                "{ns}.\u{0275}\u{0275}defineInjectable({{ token: {type_name}, factory: function {type_name}_Factory() {{ return ({use_factory})({deps}); }}, providedIn: {provided_in} }})"
            ));
        }
        return Some(format!(
            "{ns}.\u{0275}\u{0275}defineInjectable({{ token: {type_name}, factory: {use_factory}, providedIn: {provided_in} }})"
        ));
    }

    if let Some(use_existing) = get_property_source(meta, "useExisting", source) {
        return Some(format!(
            "{ns}.\u{0275}\u{0275}defineInjectable({{ token: {type_name}, factory: function {type_name}_Factory() {{ return {ns}.\u{0275}\u{0275}inject({use_existing}); }}, providedIn: {provided_in} }})"
        ));
    }

    if let Some(use_value) = get_property_source(meta, "useValue", source) {
        return Some(format!(
            "{ns}.\u{0275}\u{0275}defineInjectable({{ token: {type_name}, factory: function {type_name}_Factory() {{ return {use_value}; }}, providedIn: {provided_in} }})"
        ));
    }

    // Default: use the class factory
    Some(format!(
        "{ns}.\u{0275}\u{0275}defineInjectable({{ token: {type_name}, factory: {type_name}.\u{0275}fac, providedIn: {provided_in} }})"
    ))
}

/// Link ɵɵngDeclareInjector → ɵɵdefineInjector.
fn link_injector(
    meta: &ObjectExpression<'_>,
    source: &str,
    ns: &str,
    type_name: &str,
) -> Option<String> {
    // The injector definition uses type for consistency with other declarations
    let mut parts = vec![format!("type: {type_name}")];

    if let Some(providers) = get_property_source(meta, "providers", source) {
        parts.push(format!("providers: {providers}"));
    }
    if let Some(imports) = get_property_source(meta, "imports", source) {
        parts.push(format!("imports: {imports}"));
    }

    Some(format!("{ns}.\u{0275}\u{0275}defineInjector({{ {} }})", parts.join(", ")))
}

/// Link ɵɵngDeclareNgModule → ɵɵdefineNgModule.
fn link_ng_module(
    meta: &ObjectExpression<'_>,
    source: &str,
    ns: &str,
    type_name: &str,
) -> Option<String> {
    let mut parts = vec![format!("type: {type_name}")];

    if let Some(declarations) = get_property_source(meta, "declarations", source) {
        parts.push(format!("declarations: {declarations}"));
    }
    if let Some(imports) = get_property_source(meta, "imports", source) {
        parts.push(format!("imports: {imports}"));
    }
    if let Some(exports) = get_property_source(meta, "exports", source) {
        parts.push(format!("exports: {exports}"));
    }
    if let Some(bootstrap) = get_property_source(meta, "bootstrap", source) {
        parts.push(format!("bootstrap: {bootstrap}"));
    }
    if let Some(schemas) = get_property_source(meta, "schemas", source) {
        parts.push(format!("schemas: {schemas}"));
    }

    Some(format!("{ns}.\u{0275}\u{0275}defineNgModule({{ {} }})", parts.join(", ")))
}

/// Link ɵɵngDeclarePipe → ɵɵdefinePipe.
fn link_pipe(
    meta: &ObjectExpression<'_>,
    source: &str,
    ns: &str,
    type_name: &str,
) -> Option<String> {
    let pipe_name = get_string_property(meta, "name")?;
    let pure = get_property_source(meta, "pure", source).unwrap_or("true");
    let standalone = get_property_source(meta, "isStandalone", source).unwrap_or("true");

    Some(format!(
        "{ns}.\u{0275}\u{0275}definePipe({{ name: \"{pipe_name}\", type: {type_name}, pure: {pure}, standalone: {standalone} }})"
    ))
}

/// Link ɵɵngDeclareClassMetadata → ɵɵsetClassMetadata.
fn link_class_metadata(
    meta: &ObjectExpression<'_>,
    source: &str,
    ns: &str,
    type_name: &str,
) -> Option<String> {
    let decorators = get_property_source(meta, "decorators", source).unwrap_or("[]");
    let ctor_params = get_property_source(meta, "ctorParameters", source);
    let prop_decorators = get_property_source(meta, "propDecorators", source);

    let ctor_str = ctor_params.unwrap_or("null");
    let prop_str = prop_decorators.unwrap_or("null");

    Some(format!(
        "(() => {{ (typeof ngDevMode === \"undefined\" || ngDevMode) && {ns}.\u{0275}setClassMetadata({type_name}, {decorators}, {ctor_str}, {prop_str}); }})()"
    ))
}

/// Link ɵɵngDeclareClassMetadataAsync → ɵɵsetClassMetadataAsync.
fn link_class_metadata_async(
    meta: &ObjectExpression<'_>,
    source: &str,
    ns: &str,
    type_name: &str,
) -> Option<String> {
    let resolver_fn = get_property_source(meta, "resolveDeferredDeps", source)?;
    let decorators = get_property_source(meta, "decorators", source).unwrap_or("[]");
    let ctor_params = get_property_source(meta, "ctorParameters", source);
    let prop_decorators = get_property_source(meta, "propDecorators", source);

    let ctor_str = ctor_params.unwrap_or("null");
    let prop_str = prop_decorators.unwrap_or("null");

    Some(format!(
        "(() => {{ (typeof ngDevMode === \"undefined\" || ngDevMode) && {ns}.\u{0275}setClassMetadataAsync({type_name}, {resolver_fn}, () => {{ {ns}.\u{0275}setClassMetadata({type_name}, {decorators}, {ctor_str}, {prop_str}); }}); }})()"
    ))
}

/// Link ɵɵngDeclareDirective → ɵɵdefineDirective.
fn link_directive(
    meta: &ObjectExpression<'_>,
    source: &str,
    ns: &str,
    type_name: &str,
) -> Option<String> {
    let mut parts = vec![format!("type: {type_name}")];

    if let Some(selector) = get_string_property(meta, "selector") {
        parts.push(format!("selectors: {}", parse_selector(selector)));
    }
    if let Some(inputs) = get_property_source(meta, "inputs", source) {
        parts.push(format!("inputs: {inputs}"));
    }
    if let Some(outputs) = get_property_source(meta, "outputs", source) {
        parts.push(format!("outputs: {outputs}"));
    }
    if let Some(export_as) = get_property_source(meta, "exportAs", source) {
        parts.push(format!("exportAs: {export_as}"));
    }
    let standalone = get_bool_property(meta, "isStandalone").unwrap_or(true);
    parts.push(format!("standalone: {standalone}"));

    if let Some(host_directives) = get_property_source(meta, "hostDirectives", source) {
        parts.push(format!("hostDirectives: {host_directives}"));
    }
    if let Some(features) = get_property_source(meta, "features", source) {
        parts.push(format!("features: {features}"));
    }

    // Host bindings - convert host object to hostAttrs array
    if let Some(host_obj) = get_object_property(meta, "host") {
        let host_attrs = build_host_attrs(host_obj, source);
        if !host_attrs.is_empty() {
            parts.push(format!("hostAttrs: [{}]", host_attrs));
        }
    }

    Some(format!("{ns}.\u{0275}\u{0275}defineDirective({{ {} }})", parts.join(", ")))
}

// NOTE: link_component is intentionally not implemented.
// Component linking requires full template compilation (parsing HTML templates
// into Angular instruction sequences like ɵɵelementStart, ɵɵtext, etc.).
// This is a major feature that needs a template compiler.
// Until implemented, ɵɵngDeclareComponent is left intact for Angular's
// runtime JIT compiler to handle.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_link_factory_with_deps() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyService {
}
MyService.ɵfac = i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "20.0.0", ngImport: i0, type: MyService, deps: [], target: i0.ɵɵFactoryTarget.Injectable });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        assert!(result.code.contains("MyService_Factory"));
        assert!(!result.code.contains("ɵɵngDeclareFactory"));
    }

    #[test]
    fn test_link_factory_inherited() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyService {
}
MyService.ɵfac = i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "20.0.0", ngImport: i0, type: MyService, target: i0.ɵɵFactoryTarget.Injectable });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        assert!(result.code.contains("getInheritedFactory"));
        assert!(!result.code.contains("ɵɵngDeclareFactory"));
    }

    #[test]
    fn test_link_injectable() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyService {
}
MyService.ɵprov = i0.ɵɵngDeclareInjectable({ minVersion: "12.0.0", version: "20.0.0", ngImport: i0, type: MyService, providedIn: 'root' });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        assert!(result.code.contains("defineInjectable"));
        assert!(result.code.contains("providedIn: 'root'"));
        assert!(!result.code.contains("ɵɵngDeclareInjectable"));
    }

    #[test]
    fn test_link_class_metadata() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
i0.ɵɵngDeclareClassMetadata({ minVersion: "12.0.0", version: "20.0.0", ngImport: i0, type: MyService, decorators: [{ type: Injectable, args: [{ providedIn: 'root' }] }] });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        assert!(result.code.contains("setClassMetadata"));
        assert!(!result.code.contains("ɵɵngDeclareClassMetadata"));
    }

    #[test]
    fn test_parse_selector_tag() {
        assert_eq!(parse_selector("app-root"), r#"[["app-root"]]"#);
    }

    #[test]
    fn test_parse_selector_attribute() {
        assert_eq!(parse_selector("[ngClass]"), r#"[["", "ngClass", ""]]"#);
    }

    #[test]
    fn test_parse_selector_tag_with_attribute() {
        assert_eq!(parse_selector("div[ngClass]"), r#"[["div", "ngClass", ""]]"#);
    }

    #[test]
    fn test_parse_selector_attribute_with_value() {
        assert_eq!(parse_selector("[attr=value]"), r#"[["", "attr", "value"]]"#);
    }

    #[test]
    fn test_parse_selector_class() {
        assert_eq!(parse_selector(".my-class"), r#"[["", "class", "my-class"]]"#);
    }

    #[test]
    fn test_parse_selector_multiple() {
        assert_eq!(parse_selector("[a],[b]"), r#"[["", "a", ""], ["", "b", ""]]"#);
    }

    #[test]
    fn test_no_declarations() {
        let allocator = Allocator::default();
        let code = "console.log('hello');";
        let result = link(&allocator, code, "test.mjs");
        assert!(!result.linked);
        assert_eq!(result.code, code);
    }

    #[test]
    fn test_component_declarations_are_preserved() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyComponent {
}
MyComponent.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "20.0.0", ngImport: i0, type: MyComponent, selector: "my-comp", template: "<div>Hello</div>" });
"#;
        let result = link(&allocator, code, "test.mjs");
        // Component declarations should NOT be linked (template compilation not implemented).
        // The original ɵɵngDeclareComponent call must be preserved intact so Angular's
        // runtime can JIT-compile the template.
        assert!(!result.linked);
        assert!(result.code.contains("\u{0275}\u{0275}ngDeclareComponent"));
        assert!(!result.code.contains("defineComponent"));
    }

    #[test]
    fn test_component_preserved_while_other_declarations_linked() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyComponent {
}
MyComponent.ɵfac = i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "20.0.0", ngImport: i0, type: MyComponent, deps: [], target: i0.ɵɵFactoryTarget.Component });
MyComponent.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "20.0.0", ngImport: i0, type: MyComponent, selector: "my-comp", template: "<div>Hello</div>" });
"#;
        let result = link(&allocator, code, "test.mjs");
        // Factory should be linked but component declaration should be preserved
        assert!(result.linked);
        assert!(result.code.contains("MyComponent_Factory"));
        assert!(!result.code.contains("\u{0275}\u{0275}ngDeclareFactory"));
        assert!(result.code.contains("\u{0275}\u{0275}ngDeclareComponent"));
    }
}
