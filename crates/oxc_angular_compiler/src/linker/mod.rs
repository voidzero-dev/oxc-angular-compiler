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
    Argument, ArrayExpressionElement, CallExpression, Expression, ObjectExpression,
    ObjectPropertyKind, Program, PropertyKey, Statement,
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
    collect_declaration_edits(&program, code, filename, &mut edits);

    if edits.is_empty() {
        return LinkResult { code: code.to_string(), map: None, linked: false };
    }

    let linked_code = crate::optimizer::apply_edits(code, edits);

    LinkResult { code: linked_code, map: None, linked: true }
}

/// Recursively walk the AST to find all ɵɵngDeclare* calls and generate edits.
fn collect_declaration_edits(
    program: &Program<'_>,
    source: &str,
    filename: &str,
    edits: &mut Vec<Edit>,
) {
    for stmt in &program.body {
        walk_statement(stmt, source, filename, edits);
    }
}

/// Walk a statement looking for ɵɵngDeclare* calls.
fn walk_statement(stmt: &Statement<'_>, source: &str, filename: &str, edits: &mut Vec<Edit>) {
    match stmt {
        Statement::ExpressionStatement(expr_stmt) => {
            walk_expression(&expr_stmt.expression, source, filename, edits);
        }
        Statement::ClassDeclaration(class_decl) => {
            walk_class_body(&class_decl.body, source, filename, edits);
        }
        Statement::VariableDeclaration(var_decl) => {
            for decl in &var_decl.declarations {
                if let Some(init) = &decl.init {
                    walk_expression(init, source, filename, edits);
                }
            }
        }
        Statement::ReturnStatement(ret) => {
            if let Some(ref arg) = ret.argument {
                walk_expression(arg, source, filename, edits);
            }
        }
        Statement::BlockStatement(block) => {
            for stmt in &block.body {
                walk_statement(stmt, source, filename, edits);
            }
        }
        Statement::IfStatement(if_stmt) => {
            walk_statement(&if_stmt.consequent, source, filename, edits);
            if let Some(ref alt) = if_stmt.alternate {
                walk_statement(alt, source, filename, edits);
            }
        }
        Statement::ForStatement(for_stmt) => {
            walk_statement(&for_stmt.body, source, filename, edits);
        }
        Statement::ForInStatement(for_in) => {
            walk_statement(&for_in.body, source, filename, edits);
        }
        Statement::ForOfStatement(for_of) => {
            walk_statement(&for_of.body, source, filename, edits);
        }
        Statement::WhileStatement(while_stmt) => {
            walk_statement(&while_stmt.body, source, filename, edits);
        }
        Statement::DoWhileStatement(do_while) => {
            walk_statement(&do_while.body, source, filename, edits);
        }
        Statement::TryStatement(try_stmt) => {
            for stmt in &try_stmt.block.body {
                walk_statement(stmt, source, filename, edits);
            }
            if let Some(ref handler) = try_stmt.handler {
                for stmt in &handler.body.body {
                    walk_statement(stmt, source, filename, edits);
                }
            }
            if let Some(ref finalizer) = try_stmt.finalizer {
                for stmt in &finalizer.body {
                    walk_statement(stmt, source, filename, edits);
                }
            }
        }
        Statement::SwitchStatement(switch_stmt) => {
            for case in &switch_stmt.cases {
                for stmt in &case.consequent {
                    walk_statement(stmt, source, filename, edits);
                }
            }
        }
        Statement::LabeledStatement(labeled) => {
            walk_statement(&labeled.body, source, filename, edits);
        }
        Statement::FunctionDeclaration(func_decl) => {
            if let Some(ref body) = func_decl.body {
                for stmt in &body.statements {
                    walk_statement(stmt, source, filename, edits);
                }
            }
        }
        Statement::ExportNamedDeclaration(export_decl) => {
            if let Some(ref decl) = export_decl.declaration {
                walk_declaration(decl, source, filename, edits);
            }
        }
        Statement::ExportDefaultDeclaration(export_default) => match &export_default.declaration {
            oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class_decl) => {
                walk_class_body(&class_decl.body, source, filename, edits);
            }
            oxc_ast::ast::ExportDefaultDeclarationKind::FunctionDeclaration(func_decl) => {
                if let Some(ref body) = func_decl.body {
                    for stmt in &body.statements {
                        walk_statement(stmt, source, filename, edits);
                    }
                }
            }
            _ => {
                if let Some(expr) = export_default.declaration.as_expression() {
                    walk_expression(expr, source, filename, edits);
                }
            }
        },
        _ => {}
    }
}

/// Walk a class body looking for ɵɵngDeclare* calls in property definitions and static blocks.
fn walk_class_body(
    body: &oxc_ast::ast::ClassBody<'_>,
    source: &str,
    filename: &str,
    edits: &mut Vec<Edit>,
) {
    for element in &body.body {
        if let oxc_ast::ast::ClassElement::PropertyDefinition(prop) = element {
            if let Some(ref value) = prop.value {
                walk_expression(value, source, filename, edits);
            }
        }
        if let oxc_ast::ast::ClassElement::StaticBlock(block) = element {
            for stmt in &block.body {
                walk_statement(stmt, source, filename, edits);
            }
        }
        if let oxc_ast::ast::ClassElement::MethodDefinition(method) = element {
            if let Some(ref body) = method.value.body {
                for stmt in &body.statements {
                    walk_statement(stmt, source, filename, edits);
                }
            }
        }
    }
}

/// Walk a declaration (from export statements) looking for ɵɵngDeclare* calls.
fn walk_declaration(
    decl: &oxc_ast::ast::Declaration<'_>,
    source: &str,
    filename: &str,
    edits: &mut Vec<Edit>,
) {
    match decl {
        oxc_ast::ast::Declaration::VariableDeclaration(var_decl) => {
            for d in &var_decl.declarations {
                if let Some(init) = &d.init {
                    walk_expression(init, source, filename, edits);
                }
            }
        }
        oxc_ast::ast::Declaration::FunctionDeclaration(func_decl) => {
            if let Some(ref body) = func_decl.body {
                for stmt in &body.statements {
                    walk_statement(stmt, source, filename, edits);
                }
            }
        }
        oxc_ast::ast::Declaration::ClassDeclaration(class_decl) => {
            walk_class_body(&class_decl.body, source, filename, edits);
        }
        _ => {}
    }
}

/// Walk an expression looking for ɵɵngDeclare* calls.
fn walk_expression(expr: &Expression<'_>, source: &str, filename: &str, edits: &mut Vec<Edit>) {
    match expr {
        Expression::CallExpression(call) => {
            if let Some(name) = get_declare_name(call) {
                if let Some(edit) = link_declaration(name, call, source, filename) {
                    edits.push(edit);
                    return;
                }
            }
            // Walk arguments recursively
            for arg in &call.arguments {
                if let Argument::SpreadElement(_) = arg {
                    continue;
                }
                walk_expression(arg.to_expression(), source, filename, edits);
            }
        }
        Expression::AssignmentExpression(assign) => {
            walk_expression(&assign.right, source, filename, edits);
        }
        Expression::SequenceExpression(seq) => {
            for expr in &seq.expressions {
                walk_expression(expr, source, filename, edits);
            }
        }
        Expression::ConditionalExpression(cond) => {
            walk_expression(&cond.consequent, source, filename, edits);
            walk_expression(&cond.alternate, source, filename, edits);
        }
        Expression::LogicalExpression(logical) => {
            walk_expression(&logical.left, source, filename, edits);
            walk_expression(&logical.right, source, filename, edits);
        }
        Expression::ParenthesizedExpression(paren) => {
            walk_expression(&paren.expression, source, filename, edits);
        }
        Expression::ArrowFunctionExpression(arrow) => {
            for stmt in &arrow.body.statements {
                walk_statement(stmt, source, filename, edits);
            }
        }
        Expression::FunctionExpression(func) => {
            if let Some(body) = &func.body {
                for stmt in &body.statements {
                    walk_statement(stmt, source, filename, edits);
                }
            }
        }
        Expression::ClassExpression(class_expr) => {
            walk_class_body(&class_expr.body, source, filename, edits);
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
/// Handles both regular string literals (`"..."`) and template literals with no expressions (`` `...` ``).
fn get_string_property<'a>(obj: &'a ObjectExpression<'a>, name: &str) -> Option<&'a str> {
    for prop in &obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if matches!(&prop.key, PropertyKey::StaticIdentifier(ident) if ident.name == name) {
                match &prop.value {
                    Expression::StringLiteral(lit) => {
                        return Some(lit.value.as_str());
                    }
                    Expression::TemplateLiteral(tl) if tl.expressions.is_empty() => {
                        if let Some(quasi) = tl.quasis.first() {
                            if let Some(cooked) = &quasi.value.cooked {
                                return Some(cooked.as_str());
                            }
                        }
                    }
                    _ => {}
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
fn link_declaration(
    name: &str,
    call: &CallExpression<'_>,
    source: &str,
    filename: &str,
) -> Option<Edit> {
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
        DECLARE_COMPONENT => link_component(meta, source, filename, ns, type_name),
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

/// Convert inputs from declaration format to definition format.
///
/// Declaration format (`ɵɵngDeclareDirective`):
///   - `propertyName: "publicName"` (simple)
///   - `propertyName: ["publicName", "classPropertyName"]` (aliased)
///   - `propertyName: { classPropertyName: "...", publicName: "...", isRequired: bool,
///      isSignal: bool, transformFunction: expr }` (Angular 16+ object format)
///
/// Definition format (`ɵɵdefineDirective`):
///   - `propertyName: "publicName"` (simple, same as declaration)
///   - `propertyName: [InputFlags, "publicName", "declaredName", transform?]` (array format)
///
/// InputFlags: None=0, SignalBased=1, HasDecoratorInputTransform=2
fn convert_inputs_to_definition_format(inputs_obj: &ObjectExpression<'_>, source: &str) -> String {
    let mut entries: Vec<String> = Vec::new();

    for prop in &inputs_obj.properties {
        let ObjectPropertyKind::ObjectProperty(p) = prop else { continue };

        let key = match &p.key {
            PropertyKey::StaticIdentifier(ident) => ident.name.to_string(),
            PropertyKey::StringLiteral(s) => s.value.to_string(),
            _ => {
                // Fallback: use source text
                let span = p.span();
                entries.push(source[span.start as usize..span.end as usize].to_string());
                continue;
            }
        };

        match &p.value {
            // Simple string: propertyName: "publicName" → keep as is
            Expression::StringLiteral(lit) => {
                entries.push(format!("{key}: \"{}\"", lit.value));
            }
            // Array: check if it's declaration format [publicName, classPropertyName]
            // and convert to definition format [InputFlags, publicName, classPropertyName]
            Expression::ArrayExpression(arr) => {
                if arr.elements.len() == 2 {
                    // Check if first element is a string (declaration format)
                    let first_is_string = matches!(
                        arr.elements.first(),
                        Some(ArrayExpressionElement::StringLiteral(_))
                    );
                    if first_is_string {
                        // Declaration format: ["publicName", "classPropertyName"]
                        // Convert to: [0, "publicName", "classPropertyName"]
                        let arr_source =
                            &source[arr.span.start as usize + 1..arr.span.end as usize - 1];
                        entries.push(format!("{key}: [0, {arr_source}]"));
                    } else {
                        // Already in definition format or unknown, keep as is
                        let val =
                            &source[p.value.span().start as usize..p.value.span().end as usize];
                        entries.push(format!("{key}: {val}"));
                    }
                } else {
                    // 3+ elements likely already in definition format, keep as is
                    let val = &source[p.value.span().start as usize..p.value.span().end as usize];
                    entries.push(format!("{key}: {val}"));
                }
            }
            // Object: Angular 16+ format with classPropertyName, publicName, isRequired, etc.
            Expression::ObjectExpression(obj) => {
                let public_name = get_string_property(obj, "publicName").unwrap_or(&key);
                let declared_name = get_string_property(obj, "classPropertyName").unwrap_or(&key);
                let is_signal = get_bool_property(obj, "isSignal").unwrap_or(false);
                let is_required = get_bool_property(obj, "isRequired").unwrap_or(false);
                // Angular emits `transformFunction: null` for signal inputs without
                // transforms. Filter out "null" to avoid setting HasDecoratorInputTransform.
                let transform =
                    get_property_source(obj, "transformFunction", source).filter(|v| *v != "null");

                let mut flags = 0u32;
                if is_signal {
                    flags |= 1; // InputFlags.SignalBased
                }
                if transform.is_some() {
                    flags |= 2; // InputFlags.HasDecoratorInputTransform
                }
                // isRequired is expressed via InputFlags.SignalBased for signal inputs
                // and is checked separately for non-signal inputs
                let _ = is_required;

                if flags == 0 && transform.is_none() && public_name == declared_name {
                    // Simple case: no flags, no transform, names match
                    entries.push(format!("{key}: \"{public_name}\""));
                } else if let Some(transform_fn) = transform {
                    entries.push(format!(
                        "{key}: [{flags}, \"{public_name}\", \"{declared_name}\", {transform_fn}]"
                    ));
                } else {
                    entries
                        .push(format!("{key}: [{flags}, \"{public_name}\", \"{declared_name}\"]"));
                }
            }
            // Unknown format, keep as is
            _ => {
                let val = &source[p.value.span().start as usize..p.value.span().end as usize];
                entries.push(format!("{key}: {val}"));
            }
        }
    }

    format!("{{ {} }}", entries.join(", "))
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
    if let Some(inputs_obj) = get_object_property(meta, "inputs") {
        let converted = convert_inputs_to_definition_format(inputs_obj, source);
        parts.push(format!("inputs: {converted}"));
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

/// Extract an array expression property value from an object expression.
fn get_array_property<'a>(
    obj: &'a ObjectExpression<'a>,
    name: &str,
) -> Option<&'a oxc_ast::ast::ArrayExpression<'a>> {
    for prop in &obj.properties {
        if let ObjectPropertyKind::ObjectProperty(prop) = prop {
            if matches!(&prop.key, PropertyKey::StaticIdentifier(ident) if ident.name == name) {
                if let Expression::ArrayExpression(arr) = &prop.value {
                    return Some(arr.as_ref());
                }
            }
        }
    }
    None
}

/// Extract dependency type references from the `dependencies` array in component metadata.
///
/// In partial declarations, dependencies look like:
/// ```javascript
/// dependencies: [{ kind: "directive", type: RouterOutlet, selector: "...", ... }]
/// ```
/// Extract host properties and listeners from the `host` metadata object into a
/// `HostMetadataInput` for compilation through the full Angular expression parser.
///
/// The partial declaration format stores host bindings as:
/// ```javascript
/// host: {
///   properties: { "id": "this.dirId", "attr.aria-disabled": "disabled" },
///   listeners: { "click": "onClick($event)" }
/// }
/// ```
///
/// The values are Angular template expression strings that must be compiled through
/// the Angular expression parser (not simple string interpolation).
fn extract_host_metadata_input(
    host_obj: &ObjectExpression<'_>,
) -> crate::component::HostMetadataInput {
    let mut input = crate::component::HostMetadataInput::default();

    if let Some(properties) = get_object_property(host_obj, "properties") {
        for prop in &properties.properties {
            let ObjectPropertyKind::ObjectProperty(p) = prop else { continue };
            let key = match &p.key {
                PropertyKey::StaticIdentifier(ident) => ident.name.to_string(),
                PropertyKey::StringLiteral(s) => s.value.to_string(),
                _ => continue,
            };
            let value = match &p.value {
                Expression::StringLiteral(s) => s.value.to_string(),
                _ => continue,
            };
            input.properties.push((key, value));
        }
    }

    if let Some(listeners) = get_object_property(host_obj, "listeners") {
        for prop in &listeners.properties {
            let ObjectPropertyKind::ObjectProperty(p) = prop else { continue };
            let key = match &p.key {
                PropertyKey::StaticIdentifier(ident) => ident.name.to_string(),
                PropertyKey::StringLiteral(s) => s.value.to_string(),
                _ => continue,
            };
            let value = match &p.value {
                Expression::StringLiteral(s) => s.value.to_string(),
                _ => continue,
            };
            input.listeners.push((key, value));
        }
    }

    input
}

/// In the defineComponent output, we just need the type references:
/// ```javascript
/// dependencies: [RouterOutlet]
/// ```
fn extract_dependency_types(
    arr: &oxc_ast::ast::ArrayExpression<'_>,
    source: &str,
) -> Option<String> {
    let mut types: Vec<String> = Vec::new();
    for el in &arr.elements {
        let expr = match el {
            ArrayExpressionElement::SpreadElement(_) => continue,
            _ => el.to_expression(),
        };
        if let Expression::ObjectExpression(obj) = expr {
            if let Some(type_src) = get_property_source(obj.as_ref(), "type", source) {
                types.push(type_src.to_string());
            }
        }
    }
    if types.is_empty() { None } else { Some(format!("[{}]", types.join(", "))) }
}

/// Build a query function (contentQueries or viewQuery) from query metadata.
///
/// Content query metadata format:
/// ```javascript
/// { propertyName: "items", first: true, predicate: SomeType, descendants: true }
/// ```
///
/// View query metadata format:
/// ```javascript
/// { propertyName: "child", first: true, predicate: SomeType, static: true }
/// ```
fn build_queries(
    queries: &oxc_ast::ast::ArrayExpression<'_>,
    source: &str,
    ns: &str,
    type_name: &str,
    is_content_query: bool,
) -> Option<String> {
    if queries.elements.is_empty() {
        return None;
    }

    let mut create_stmts: Vec<String> = Vec::new();
    let mut update_stmts: Vec<String> = Vec::new();
    let mut t_declared = false;

    for el in &queries.elements {
        let expr = match el {
            ArrayExpressionElement::SpreadElement(_) => continue,
            _ => el.to_expression(),
        };
        let Expression::ObjectExpression(query_obj) = expr else { continue };

        let prop_name =
            get_string_property(query_obj.as_ref(), "propertyName").unwrap_or("unknown");
        let first = get_bool_property(query_obj.as_ref(), "first").unwrap_or(false);
        let is_static = get_bool_property(query_obj.as_ref(), "static").unwrap_or(false);
        let descendants = get_bool_property(query_obj.as_ref(), "descendants").unwrap_or(false);
        let is_signal = get_bool_property(query_obj.as_ref(), "isSignal").unwrap_or(false);
        let read = get_property_source(query_obj.as_ref(), "read", source);

        // Build predicate - can be a type reference or string array
        let predicate =
            get_property_source(query_obj.as_ref(), "predicate", source).unwrap_or("null");

        // Calculate flags: DESCENDANTS=1, IS_STATIC=2, EMIT_DISTINCT_CHANGES_ONLY=4
        // View queries always have descendants=true; content queries read it from metadata.
        let has_descendants = if is_content_query { descendants } else { true };
        let mut flags = 4u32; // EMIT_DISTINCT_CHANGES_ONLY (always on)
        if has_descendants {
            flags |= 1; // DESCENDANTS
        }
        if is_static {
            flags |= 2; // IS_STATIC
        }

        // Create block — signal queries use different instructions with ctx.propertyName
        if is_content_query {
            if is_signal {
                let mut args = format!("dirIndex, ctx.{prop_name}, {predicate}, {flags}");
                if let Some(read_expr) = read {
                    args = format!("{args}, {read_expr}");
                }
                create_stmts.push(format!("{ns}.\u{0275}\u{0275}contentQuerySignal({args})"));
            } else {
                let mut args = format!("dirIndex, {predicate}, {flags}");
                if let Some(read_expr) = read {
                    args = format!("{args}, {read_expr}");
                }
                create_stmts.push(format!("{ns}.\u{0275}\u{0275}contentQuery({args})"));
            }
        } else if is_signal {
            let mut args = format!("ctx.{prop_name}, {predicate}, {flags}");
            if let Some(read_expr) = read {
                args = format!("{args}, {read_expr}");
            }
            create_stmts.push(format!("{ns}.\u{0275}\u{0275}viewQuerySignal({args})"));
        } else {
            let mut args = format!("{predicate}, {flags}");
            if let Some(read_expr) = read {
                args = format!("{args}, {read_expr}");
            }
            create_stmts.push(format!("{ns}.\u{0275}\u{0275}viewQuery({args})"));
        }

        // Update block — signal queries just advance; regular queries refresh+assign
        if is_signal {
            update_stmts.push(format!("{ns}.\u{0275}\u{0275}queryAdvance()"));
        } else {
            let t_var = if !t_declared {
                t_declared = true;
                "let _t;\n"
            } else {
                ""
            };
            let access = if first { ".first" } else { "" };
            update_stmts.push(format!(
                "{t_var}{ns}.\u{0275}\u{0275}queryRefresh(_t = {ns}.\u{0275}\u{0275}loadQuery()) && (ctx.{prop_name} = _t{access})"
            ));
        }
    }

    let create_block = create_stmts.join(";\n");
    let update_block = update_stmts.join(";\n");

    if is_content_query {
        Some(format!(
            "function {type_name}_ContentQueries(rf, ctx, dirIndex) {{\nif (rf & 1) {{\n{create_block};\n}}\nif (rf & 2) {{\n{update_block};\n}}\n}}"
        ))
    } else {
        Some(format!(
            "function {type_name}_Query(rf, ctx) {{\nif (rf & 1) {{\n{create_block};\n}}\nif (rf & 2) {{\n{update_block};\n}}\n}}"
        ))
    }
}

/// Build the features array from component metadata.
///
/// Examines boolean flags and providers to build the features array:
/// - `usesInheritance: true` → `ns.ɵɵInheritDefinitionFeature`
/// - `usesOnChanges: true` → `ns.ɵɵNgOnChangesFeature`
/// - `providers: [...]` → `ns.ɵɵProvidersFeature([...])`
/// Order is important: ProvidersFeature → InheritDefinitionFeature → NgOnChangesFeature
/// (see definition.rs line 990 and packages/compiler/src/render3/view/compiler.ts:119-161)
fn build_features(meta: &ObjectExpression<'_>, source: &str, ns: &str) -> Option<String> {
    let mut features: Vec<String> = Vec::new();

    // 1. ProvidersFeature — must come before InheritDefinitionFeature
    let providers = get_property_source(meta, "providers", source);
    let view_providers = get_property_source(meta, "viewProviders", source);
    match (providers, view_providers) {
        (Some(p), Some(vp)) => {
            features.push(format!("{ns}.\u{0275}\u{0275}ProvidersFeature({p}, {vp})"));
        }
        (Some(p), None) => {
            features.push(format!("{ns}.\u{0275}\u{0275}ProvidersFeature({p})"));
        }
        (None, Some(vp)) => {
            features.push(format!("{ns}.\u{0275}\u{0275}ProvidersFeature([], {vp})"));
        }
        (None, None) => {}
    }

    // 2. InheritDefinitionFeature
    if get_bool_property(meta, "usesInheritance") == Some(true) {
        features.push(format!("{ns}.\u{0275}\u{0275}InheritDefinitionFeature"));
    }

    // 3. NgOnChangesFeature
    if get_bool_property(meta, "usesOnChanges") == Some(true) {
        features.push(format!("{ns}.\u{0275}\u{0275}NgOnChangesFeature"));
    }

    if features.is_empty() { None } else { Some(format!("[{}]", features.join(", "))) }
}

/// Link ɵɵngDeclareComponent → ɵɵdefineComponent.
///
/// A component extends a directive with template compilation and additional
/// component-specific metadata (styles, encapsulation, change detection, etc.).
///
/// The replacement is wrapped in an IIFE to scope the template function declarations:
/// ```javascript
/// (() => {
///   function Child_Template(rf, ctx) { ... }
///   function Component_Template(rf, ctx) { ... }
///   return i0.ɵɵdefineComponent({ ... template: Component_Template, ... });
/// })()
/// ```
fn link_component(
    meta: &ObjectExpression<'_>,
    source: &str,
    filename: &str,
    ns: &str,
    type_name: &str,
) -> Option<String> {
    // Extract template string - required for component linking
    let template = get_string_property(meta, "template")?;
    let preserve_whitespaces = get_bool_property(meta, "preserveWhitespaces").unwrap_or(false);

    // Compile the template using the full template compilation pipeline.
    let template_allocator = Allocator::default();

    // We need to leak the template string into the template allocator's lifetime
    let template_owned: String = template.to_string();
    let template_ref: &str = template_allocator.alloc_str(&template_owned);

    let template_output = crate::component::compile_template_for_linker(
        &template_allocator,
        template_ref,
        type_name,
        filename,
        preserve_whitespaces,
    )
    .ok()?;

    // Build the defineComponent properties
    let mut parts: Vec<String> = Vec::new();

    // 1. type
    parts.push(format!("type: {type_name}"));

    // 2. selectors
    if let Some(selector) = get_string_property(meta, "selector") {
        parts.push(format!("selectors: {}", parse_selector(selector)));
    }

    // 3. contentQueries
    if let Some(queries_arr) = get_array_property(meta, "queries") {
        if let Some(cq_fn) = build_queries(queries_arr, source, ns, type_name, true) {
            parts.push(format!("contentQueries: {cq_fn}"));
        }
    }

    // 4. viewQuery
    if let Some(view_queries_arr) = get_array_property(meta, "viewQueries") {
        if let Some(vq_fn) = build_queries(view_queries_arr, source, ns, type_name, false) {
            parts.push(format!("viewQuery: {vq_fn}"));
        }
    }

    // 5-7. Host bindings (hostAttrs, hostVars, hostBindings)
    if let Some(host_obj) = get_object_property(meta, "host") {
        // Static attributes → hostAttrs
        let host_attrs = build_host_attrs(host_obj, source);
        if !host_attrs.is_empty() {
            parts.push(format!("hostAttrs: [{host_attrs}]"));
        }

        // Dynamic bindings → hostVars + hostBindings function
        // Extract host properties and listeners as HostMetadataInput and compile
        // through the full Angular expression parser for correct output.
        let host_input = extract_host_metadata_input(host_obj);
        let selector = get_string_property(meta, "selector");
        if let Some((host_fn, host_vars)) =
            crate::component::compile_host_bindings_for_linker(&host_input, type_name, selector)
        {
            if host_vars > 0 {
                parts.push(format!("hostVars: {host_vars}"));
            }
            parts.push(format!("hostBindings: {host_fn}"));
        }
    }

    // 8. inputs
    if let Some(inputs_obj) = get_object_property(meta, "inputs") {
        let converted = convert_inputs_to_definition_format(inputs_obj, source);
        parts.push(format!("inputs: {converted}"));
    }

    // 9. outputs
    if let Some(outputs) = get_property_source(meta, "outputs", source) {
        parts.push(format!("outputs: {outputs}"));
    }

    // 10. exportAs
    if let Some(export_as) = get_property_source(meta, "exportAs", source) {
        parts.push(format!("exportAs: {export_as}"));
    }

    // 11. standalone
    let standalone = get_bool_property(meta, "isStandalone").unwrap_or(true);
    parts.push(format!("standalone: {standalone}"));

    // 11b. hostDirectives (Directive Composition API)
    if let Some(host_directives) = get_property_source(meta, "hostDirectives", source) {
        parts.push(format!("hostDirectives: {host_directives}"));
    }

    // 12. features
    if let Some(features) = build_features(meta, source, ns) {
        parts.push(format!("features: {features}"));
    }

    // 13. ngContentSelectors (from template compilation)
    if let Some(ref ng_content_selectors) = template_output.ng_content_selectors_js {
        parts.push(format!("ngContentSelectors: {ng_content_selectors}"));
    }

    // 14. decls (from template compilation)
    parts.push(format!("decls: {}", template_output.decls));

    // 15. vars (from template compilation)
    parts.push(format!("vars: {}", template_output.vars));

    // 16. consts (from template compilation)
    if let Some(ref consts) = template_output.consts_js {
        parts.push(format!("consts: {consts}"));
    }

    // 17. template (reference to the compiled function)
    parts.push(format!("template: {}", template_output.template_fn_name));

    // 18. dependencies (extract type references from dependency objects)
    if let Some(deps_arr) = get_array_property(meta, "dependencies") {
        if let Some(deps_str) = extract_dependency_types(deps_arr, source) {
            parts.push(format!("dependencies: {deps_str}"));
        }
    }

    // 19-20. styles + encapsulation (interdependent)
    // Determine encapsulation mode: Emulated is the default
    let is_emulated = match get_property_source(meta, "encapsulation", source) {
        Some(encap) if encap.contains("None") => false,
        Some(encap) if encap.contains("ShadowDom") => false,
        _ => true, // Emulated is the default
    };
    let is_shadow_dom = matches!(
        get_property_source(meta, "encapsulation", source),
        Some(encap) if encap.contains("ShadowDom")
    );

    // Process styles: apply CSS scoping for Emulated encapsulation
    let mut has_styles = false;
    if let Some(styles_arr) = get_array_property(meta, "styles") {
        let mut scoped_styles: Vec<String> = Vec::new();
        for el in &styles_arr.elements {
            let expr = match el {
                ArrayExpressionElement::SpreadElement(_) => continue,
                _ => el.to_expression(),
            };
            if let Expression::StringLiteral(s) = expr {
                let style = s.value.as_str();
                if is_emulated {
                    let scoped =
                        crate::styles::shim_css_text(style, "_ngcontent-%COMP%", "_nghost-%COMP%");
                    if !scoped.trim().is_empty() {
                        scoped_styles.push(crate::output::emitter::escape_string(&scoped, false));
                    }
                } else if !style.trim().is_empty() {
                    scoped_styles.push(crate::output::emitter::escape_string(style, false));
                }
            }
        }
        if !scoped_styles.is_empty() {
            has_styles = true;
            parts.push(format!("styles: [{}]", scoped_styles.join(", ")));
        }
    }

    // Encapsulation: downgrade Emulated → None when no styles
    // (per Angular compiler.ts: "If there is no style, don't generate css selectors on elements")
    if is_shadow_dom {
        parts.push("encapsulation: 3".to_string());
    } else if !is_emulated {
        // Explicitly set to None
        parts.push("encapsulation: 2".to_string());
    } else if !has_styles {
        // Emulated with no styles → downgrade to None
        parts.push("encapsulation: 2".to_string());
    }
    // else: Emulated with styles is the default (0), no need to emit

    // 21. data (animations)
    if let Some(animations) = get_property_source(meta, "animations", source) {
        parts.push(format!("data: {{ animation: {animations} }}"));
    }

    // 22. changeDetection
    if let Some(cd) = get_property_source(meta, "changeDetection", source) {
        if cd.contains("OnPush") {
            parts.push("changeDetection: 0".to_string());
        }
        // Default (1) is the default, no need to emit
    }

    let define_component =
        format!("{ns}.\u{0275}\u{0275}defineComponent({{ {} }})", parts.join(", "));

    // Wrap in IIFE with template declarations
    let declarations = &template_output.declarations_js;
    if declarations.trim().is_empty() {
        Some(define_component)
    } else {
        Some(format!("(() => {{\n{declarations}\nreturn {define_component};\n}})()"))
    }
}

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
    fn test_link_directive_with_aliased_inputs() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class RxFor {
}
RxFor.ɵdir = i0.ɵɵngDeclareDirective({ minVersion: "14.0.0", version: "16.2.10", ngImport: i0, type: RxFor, isStandalone: true, selector: "[rxFor][rxForOf]", inputs: { rxForOf: "rxForOf", renderParent: ["rxForParent", "renderParent"], trackBy: ["rxForTrackBy", "trackBy"] } });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        assert!(result.code.contains("defineDirective"));
        assert!(!result.code.contains("ɵɵngDeclareDirective"));
        // Simple inputs should stay as string: rxForOf: "rxForOf"
        assert!(result.code.contains(r#"rxForOf: "rxForOf""#));
        // Aliased inputs must be converted with InputFlags prepended:
        // ["rxForTrackBy", "trackBy"] → [0, "rxForTrackBy", "trackBy"]
        assert!(
            result.code.contains(r#"trackBy: [0, "rxForTrackBy", "trackBy"]"#),
            "Expected trackBy to have InputFlags prepended. Got: {}",
            result.code
        );
        assert!(
            result.code.contains(r#"renderParent: [0, "rxForParent", "renderParent"]"#),
            "Expected renderParent to have InputFlags prepended. Got: {}",
            result.code
        );
    }

    #[test]
    fn test_link_component_basic() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyComponent {
}
MyComponent.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "20.0.0", ngImport: i0, type: MyComponent, selector: "my-comp", template: "<div>Hello</div>" });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked, "Component should be linked");
        assert!(
            result.code.contains("defineComponent"),
            "Should contain defineComponent, got:\n{}",
            result.code
        );
        assert!(
            !result.code.contains("\u{0275}\u{0275}ngDeclareComponent"),
            "Should not contain ngDeclareComponent, got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("MyComponent_Template"),
            "Should contain compiled template function, got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("selectors: [[\"my-comp\"]]"),
            "Should contain parsed selectors, got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_link_component_with_factory() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyComponent {
}
MyComponent.ɵfac = i0.ɵɵngDeclareFactory({ minVersion: "12.0.0", version: "20.0.0", ngImport: i0, type: MyComponent, deps: [], target: i0.ɵɵFactoryTarget.Component });
MyComponent.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "20.0.0", ngImport: i0, type: MyComponent, selector: "my-comp", template: "<div>Hello</div>" });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        assert!(result.code.contains("MyComponent_Factory"));
        assert!(!result.code.contains("\u{0275}\u{0275}ngDeclareFactory"));
        assert!(result.code.contains("defineComponent"));
        assert!(!result.code.contains("\u{0275}\u{0275}ngDeclareComponent"));
    }

    #[test]
    fn test_link_component_with_dependencies() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
import * as i1 from "@angular/router";
class EmptyOutletComponent {
}
EmptyOutletComponent.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "20.0.0", ngImport: i0, type: EmptyOutletComponent, selector: "ng-component", template: "<router-outlet/>", isStandalone: true, dependencies: [{ kind: "directive", type: i1.RouterOutlet, selector: "router-outlet" }] });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked, "Should be linked");
        assert!(
            result.code.contains("defineComponent"),
            "Should contain defineComponent, got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("dependencies: [i1.RouterOutlet]"),
            "Should extract dependency types, got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_link_component_with_features() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyComponent {
}
MyComponent.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "20.0.0", ngImport: i0, type: MyComponent, selector: "my-comp", template: "<div></div>", usesInheritance: true, providers: [SomeService] });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        assert!(
            result.code.contains("InheritDefinitionFeature"),
            "Should have InheritDefinitionFeature, got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("ProvidersFeature"),
            "Should have ProvidersFeature, got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_link_component_with_ng_content() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class CdkStep {
}
CdkStep.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "20.0.0", ngImport: i0, type: CdkStep, selector: "cdk-step", template: "<ng-template><ng-content></ng-content></ng-template>", isStandalone: true });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        assert!(
            result.code.contains("defineComponent"),
            "Should contain defineComponent, got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("ngContentSelectors"),
            "Should contain ngContentSelectors for ng-content, got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_link_component_with_encapsulation() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyComponent {
}
MyComponent.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "20.0.0", ngImport: i0, type: MyComponent, selector: "my-comp", template: "<div></div>", encapsulation: i0.ViewEncapsulation.None });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        assert!(
            result.code.contains("encapsulation: 2"),
            "ViewEncapsulation.None should be 2, got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_link_component_with_change_detection() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyComponent {
}
MyComponent.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "20.0.0", ngImport: i0, type: MyComponent, selector: "my-comp", template: "<div></div>", changeDetection: i0.ChangeDetectionStrategy.OnPush });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        assert!(
            result.code.contains("changeDetection: 0"),
            "ChangeDetectionStrategy.OnPush should be 0, got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_link_component_with_host_attrs() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyComponent {
}
MyComponent.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "20.0.0", ngImport: i0, type: MyComponent, selector: "my-comp", template: "<div></div>", host: { attributes: { "role": "tree" }, classAttribute: "cdk-tree" } });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        assert!(
            result.code.contains("hostAttrs:"),
            "Should contain hostAttrs, got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("\"role\""),
            "Should contain role attribute, got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_link_component_with_host_bindings() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyComponent {
}
MyComponent.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "20.0.0", ngImport: i0, type: MyComponent, selector: "my-comp", template: "<div></div>", host: { properties: { "id": "this.dirId", "attr.aria-disabled": "disabled" }, listeners: { "click": "onClick($event)" } } });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        // Should have hostVars for the 2 property bindings
        assert!(
            result.code.contains("hostVars:"),
            "Should contain hostVars, got:\n{}",
            result.code
        );
        // Should have hostBindings function
        assert!(
            result.code.contains("hostBindings:"),
            "Should contain hostBindings, got:\n{}",
            result.code
        );
        // The host binding function should properly compile expressions, not raw strings with quotes
        assert!(
            !result.code.contains(r#"ctx."this.dirId""#),
            "Should NOT contain invalid ctx.\"this.dirId\" expression, got:\n{}",
            result.code
        );
        // Should have proper context property access
        assert!(
            result.code.contains("ctx.dirId"),
            "Should contain properly compiled ctx.dirId, got:\n{}",
            result.code
        );
        // Listener should be properly compiled (not raw string with quotes)
        assert!(
            !result.code.contains(r#"ctx."onClick($event)""#),
            "Should NOT contain invalid listener expression, got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_features_order_providers_before_inherit() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyComp {
}
MyComp.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "20.0.0", ngImport: i0, type: MyComp, selector: "my-comp", providers: [SomeProvider], usesInheritance: true, usesOnChanges: true, template: "<div></div>" });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        let code = &result.code;
        // Canonical order: ProvidersFeature → InheritDefinitionFeature → NgOnChangesFeature
        let providers_pos = code.find("ProvidersFeature").expect("should have ProvidersFeature");
        let inherit_pos =
            code.find("InheritDefinitionFeature").expect("should have InheritDefinitionFeature");
        let on_changes_pos =
            code.find("NgOnChangesFeature").expect("should have NgOnChangesFeature");
        assert!(
            providers_pos < inherit_pos,
            "ProvidersFeature must come before InheritDefinitionFeature"
        );
        assert!(
            inherit_pos < on_changes_pos,
            "InheritDefinitionFeature must come before NgOnChangesFeature"
        );
    }

    #[test]
    fn test_signal_input_null_transform_no_flag() {
        let allocator = Allocator::default();
        // Angular emits `transformFunction: null` for signal inputs without transforms.
        // This must NOT set the HasDecoratorInputTransform flag (2).
        let code = r#"
import * as i0 from "@angular/core";
class MyComp {
}
MyComp.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "17.0.0", version: "20.0.0", ngImport: i0, type: MyComp, selector: "my-comp", inputs: { name: { classPropertyName: "name", publicName: "name", isSignal: true, isRequired: true, transformFunction: null } }, template: "<div></div>" });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked);
        // Signal input flag = 1, NOT 3 (1 | 2). Must not include HasDecoratorInputTransform.
        assert!(
            result.code.contains(r#"name: [1, "name", "name"]"#),
            "Signal input with null transform should have flags=1 (SignalBased only), got:\n{}",
            result.code
        );
        assert!(!result.code.contains("null]"), "Should not include null transform in output");
    }

    #[test]
    fn test_link_component_with_template_literal() {
        let allocator = Allocator::default();
        let code = r#"
import * as i0 from "@angular/core";
class MyComponent {
}
MyComponent.ɵcmp = i0.ɵɵngDeclareComponent({ minVersion: "14.0.0", version: "21.0.6", ngImport: i0, type: MyComponent, selector: "my-comp", template: `<div>Hello</div>`, isInline: true });
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked, "Component with template literal should be linked");
        assert!(
            result.code.contains("defineComponent"),
            "Should contain defineComponent, got:\n{}",
            result.code
        );
        assert!(
            !result.code.contains("\u{0275}\u{0275}ngDeclareComponent"),
            "Should not contain ngDeclareComponent, got:\n{}",
            result.code
        );
    }

    #[test]
    fn test_link_component_with_template_literal_static_field() {
        let allocator = Allocator::default();
        // This matches Angular 21's actual output format for @angular/router's ɵEmptyOutletComponent
        let code = r#"
import * as i0 from "@angular/core";
class EmptyOutletComponent {
  static ɵfac = i0.ɵɵngDeclareFactory({
    minVersion: "12.0.0",
    version: "21.0.6",
    ngImport: i0,
    type: EmptyOutletComponent,
    deps: [],
    target: i0.ɵɵFactoryTarget.Component
  });
  static ɵcmp = i0.ɵɵngDeclareComponent({
    minVersion: "14.0.0",
    version: "21.0.6",
    type: EmptyOutletComponent,
    isStandalone: true,
    selector: "ng-component",
    exportAs: ["emptyRouterOutlet"],
    ngImport: i0,
    template: `<router-outlet />`,
    isInline: true,
    dependencies: [{
      kind: "directive",
      type: RouterOutlet,
      selector: "router-outlet",
      inputs: ["name", "routerOutletData"],
      outputs: ["activate", "deactivate", "attach", "detach"],
      exportAs: ["outlet"]
    }]
  });
}
"#;
        let result = link(&allocator, code, "test.mjs");
        assert!(result.linked, "Component with template literal in static field should be linked");
        assert!(
            result.code.contains("defineComponent"),
            "Should contain defineComponent, got:\n{}",
            result.code
        );
        assert!(
            !result.code.contains("\u{0275}\u{0275}ngDeclareComponent"),
            "Should not contain ngDeclareComponent, got:\n{}",
            result.code
        );
        assert!(
            result.code.contains("dependencies: [RouterOutlet]"),
            "Should extract dependency types, got:\n{}",
            result.code
        );
    }
}
