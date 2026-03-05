//! Angular property decorator parsing.
//!
//! This module extracts metadata from property-level Angular decorators:
//! - `@Input()` - for input property bindings
//! - `@Output()` - for output event bindings
//! - `@ViewChild()` / `@ViewChildren()` - for view queries
//! - `@ContentChild()` / `@ContentChildren()` - for content queries
//! - `@HostBinding()` - for host property bindings
//! - `@HostListener()` - for host event listeners
//!
//! These decorators are found on class properties and methods, and define
//! how the directive/component interacts with its parent context.

use oxc_allocator::{Allocator, Vec};
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, Class, ClassElement, Decorator, Expression,
    MethodDefinitionKind, ObjectPropertyKind, PropertyKey,
};
use oxc_span::Atom;

use super::metadata::{QueryPredicate, R3InputMetadata, R3QueryMetadata};
use crate::output::ast::OutputExpression;
use crate::output::oxc_converter::convert_oxc_expression;

// ============================================================================
// Helper Functions
// ============================================================================

/// Find a decorator by name from a list of decorators.
///
/// Searches for decorators that are either:
/// - Simple identifiers: `@Input`
/// - Call expressions: `@Input()` or `@Input('alias')`
///
/// Returns the first matching decorator.
fn find_decorator_by_name<'a>(
    decorators: &'a oxc_allocator::Vec<'a, Decorator<'a>>,
    name: &str,
) -> Option<&'a Decorator<'a>> {
    decorators.iter().find(|d| match &d.expression {
        Expression::CallExpression(call) => match &call.callee {
            Expression::Identifier(id) => id.name == name,
            _ => false,
        },
        Expression::Identifier(id) => id.name == name,
        _ => false,
    })
}

/// Get the property key name as an Atom.
///
/// Handles both identifier keys and string literal keys.
fn get_property_key_name<'a>(key: &PropertyKey<'a>) -> Option<Atom<'a>> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.clone().into()),
        PropertyKey::StringLiteral(lit) => Some(lit.value.clone()),
        _ => None,
    }
}

/// Extract a string value from an expression.
///
/// Handles string literals and simple template literals (no expressions).
fn extract_string_value<'a>(expr: &Expression<'a>) -> Option<Atom<'a>> {
    match expr {
        Expression::StringLiteral(lit) => Some(lit.value.clone()),
        Expression::TemplateLiteral(tpl) if tpl.expressions.is_empty() => {
            tpl.quasis.first().and_then(|q| q.value.cooked.clone())
        }
        _ => None,
    }
}

/// Extract a boolean value from an expression.
fn extract_boolean_value(expr: &Expression<'_>) -> Option<bool> {
    match expr {
        Expression::BooleanLiteral(lit) => Some(lit.value),
        _ => None,
    }
}

/// Try to unwrap a forwardRef call and extract the inner expression.
///
/// For `forwardRef(() => MyClass)`, returns `Some(MyClass expression)`.
/// For non-forwardRef expressions, returns None.
fn try_unwrap_forward_ref<'a>(expr: &'a Expression<'a>) -> Option<&'a Expression<'a>> {
    let call = match expr {
        Expression::CallExpression(call) => call,
        _ => return None,
    };

    // Check if callee is forwardRef
    let is_forward_ref =
        matches!(&call.callee, Expression::Identifier(id) if id.name == "forwardRef");
    if !is_forward_ref {
        return None;
    }

    // Get the first argument (should be an arrow function)
    let first_arg = call.arguments.first()?;
    let arrow = match first_arg {
        Argument::ArrowFunctionExpression(arrow) => arrow,
        _ => return None,
    };

    // Handle expression body: () => MyClass
    if arrow.expression {
        // For expression body, the statement is the expression wrapped in ExpressionStatement
        let stmt = arrow.body.statements.first()?;
        if let oxc_ast::ast::Statement::ExpressionStatement(expr_stmt) = stmt {
            return Some(&expr_stmt.expression);
        }
    } else if arrow.body.statements.len() == 1 {
        // Handle explicit return: () => { return MyClass; }
        let stmt = arrow.body.statements.first()?;
        if let oxc_ast::ast::Statement::ReturnStatement(ret) = stmt {
            return ret.argument.as_ref();
        }
    }

    None
}

// ============================================================================
// @Input Decorator Parsing
// ============================================================================

/// Parsed @Input decorator configuration.
struct InputConfig<'a> {
    /// Alias name for the input binding (different from property name).
    alias: Option<Atom<'a>>,
    /// Whether this input is required.
    required: bool,
    /// Transform function for the input value.
    transform: Option<OutputExpression<'a>>,
}

impl<'a> Default for InputConfig<'a> {
    fn default() -> Self {
        Self { alias: None, required: false, transform: None }
    }
}

/// Parse the configuration from an @Input decorator.
///
/// Handles these variants:
/// - `@Input()` - no configuration
/// - `@Input('alias')` - string alias
/// - `@Input({ alias: 'name', required: true, transform: fn })` - full config
fn parse_input_config<'a>(
    allocator: &'a Allocator,
    decorator: &'a Decorator<'a>,
) -> InputConfig<'a> {
    let Expression::CallExpression(call) = &decorator.expression else {
        return InputConfig::default();
    };

    let Some(first_arg) = call.arguments.first() else {
        return InputConfig::default();
    };

    match first_arg {
        // @Input('alias')
        Argument::StringLiteral(lit) => {
            InputConfig { alias: Some(lit.value.clone()), ..Default::default() }
        }

        // @Input({ alias: 'name', required: true, transform: fn })
        Argument::ObjectExpression(obj) => {
            let mut config = InputConfig::default();

            for prop in &obj.properties {
                if let ObjectPropertyKind::ObjectProperty(prop) = prop {
                    let Some(key_name) = get_property_key_name(&prop.key) else {
                        continue;
                    };

                    match key_name.as_str() {
                        "alias" => {
                            config.alias = extract_string_value(&prop.value);
                        }
                        "required" => {
                            config.required = extract_boolean_value(&prop.value).unwrap_or(false);
                        }
                        "transform" => {
                            config.transform = convert_oxc_expression(allocator, &prop.value);
                        }
                        _ => {}
                    }
                }
            }

            config
        }

        _ => InputConfig::default(),
    }
}

/// Metadata for a model() signal, which creates both an input and an output.
///
/// Based on Angular's `ModelMapping` in `model_function.ts`.
struct ModelMapping<'a> {
    /// The input metadata (signal-based).
    input: R3InputMetadata<'a>,
    /// The output metadata (class property name, binding property name).
    /// Output binding name is always `inputName + "Change"`.
    output: (Atom<'a>, Atom<'a>),
}

/// Try to detect and parse a signal-based model from a property initializer.
///
/// Signal-based models are created by calling `model()` or `model.required()`
/// from `@angular/core`. They create both an input AND an output.
///
/// # Examples
/// ```typescript
/// readonly open = model(false);           // creates input 'open' and output 'openChange'
/// readonly count = model.required<number>(); // creates required input 'count' and output 'countChange'
/// readonly aliased = model<string>(undefined, { alias: 'myAlias' }); // input alias 'myAlias', output 'myAliasChange'
/// ```
///
/// Based on Angular's `model_function.ts` in the compiler-cli.
fn try_parse_signal_model<'a>(
    allocator: &'a Allocator,
    value: &Expression<'a>,
    property_name: Atom<'a>,
) -> Option<ModelMapping<'a>> {
    // Check if the value is a call expression
    let call_expr = match value {
        Expression::CallExpression(call) => call,
        _ => return None,
    };

    // Determine if this is model() or model.required()
    let is_required = match &call_expr.callee {
        // model() - simple identifier call
        Expression::Identifier(id) if id.name == "model" => false,
        // model.required() - member expression call
        Expression::StaticMemberExpression(member) => {
            // Check for model.required
            if member.property.name == "required" {
                match &member.object {
                    Expression::Identifier(id) if id.name == "model" => true,
                    _ => return None,
                }
            } else if member.property.name == "model" {
                // Handle namespaced calls like `core.model()`
                if let Expression::Identifier(_) = &member.object {
                    let output_binding = Atom::from(
                        allocator.alloc_str(&format!("{}Change", property_name.as_str())),
                    );
                    return Some(ModelMapping {
                        input: R3InputMetadata {
                            class_property_name: property_name.clone(),
                            binding_property_name: property_name.clone(),
                            required: false,
                            is_signal: true,
                            transform_function: None,
                        },
                        output: (property_name, output_binding),
                    });
                }
                return None;
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Parse options from arguments
    // For model(): first arg is initial value, second arg is options
    // For model.required(): first arg is options
    let options_arg_index = if is_required { 0 } else { 1 };

    let mut alias: Option<Atom<'a>> = None;

    if let Some(options_arg) = call_expr.arguments.get(options_arg_index) {
        if let Argument::ObjectExpression(obj) = options_arg {
            for prop in &obj.properties {
                if let ObjectPropertyKind::ObjectProperty(prop) = prop {
                    let Some(key_name) = get_property_key_name(&prop.key) else {
                        continue;
                    };

                    if key_name.as_str() == "alias" {
                        alias = extract_string_value(&prop.value);
                    }
                }
            }
        }
    }

    let binding_property_name = alias.unwrap_or_else(|| property_name.clone());
    // Output binding name is always `bindingPropertyName + "Change"`
    let output_binding_name =
        Atom::from(allocator.alloc_str(&format!("{}Change", binding_property_name.as_str())));

    Some(ModelMapping {
        input: R3InputMetadata {
            class_property_name: property_name.clone(),
            binding_property_name,
            required: is_required,
            is_signal: true,
            transform_function: None,
        },
        output: (property_name, output_binding_name),
    })
}

/// Try to detect and parse a signal-based output from a property initializer.
///
/// Signal-based outputs are created by calling `output()` or `output<T>()`
/// from `@angular/core`. Unlike `model()`, they only create an output (no input).
///
/// # Examples
/// ```typescript
/// readonly openChange = output<boolean>(); // creates output 'openChange'
/// readonly clicked = output();             // creates output 'clicked'
/// readonly aliased = output<string>({ alias: 'myAlias' }); // creates output 'myAlias'
/// ```
///
/// Based on Angular's `output_function.ts` in the compiler-cli.
fn try_parse_signal_output<'a>(
    value: &Expression<'a>,
    property_name: Atom<'a>,
) -> Option<(Atom<'a>, Atom<'a>)> {
    // Check if the value is a call expression
    let call_expr = match value {
        Expression::CallExpression(call) => call,
        _ => return None,
    };

    // Check if this is output() - note that output() does NOT support .required()
    let is_output = match &call_expr.callee {
        // output() - simple identifier call
        Expression::Identifier(id) if id.name == "output" => true,
        // Handle namespaced calls like `core.output()`
        Expression::StaticMemberExpression(member) => {
            if member.property.name == "output" {
                matches!(&member.object, Expression::Identifier(_))
            } else {
                false
            }
        }
        _ => false,
    };

    if !is_output {
        return None;
    }

    // Parse options from the first argument (options are the first arg for output())
    // Options can contain an alias
    let mut alias: Option<Atom<'a>> = None;

    if let Some(first_arg) = call_expr.arguments.first() {
        if let Argument::ObjectExpression(obj) = first_arg {
            for prop in &obj.properties {
                if let ObjectPropertyKind::ObjectProperty(prop) = prop {
                    let Some(key_name) = get_property_key_name(&prop.key) else {
                        continue;
                    };

                    if key_name.as_str() == "alias" {
                        alias = extract_string_value(&prop.value);
                    }
                }
            }
        }
    }

    let binding_property_name = alias.unwrap_or_else(|| property_name.clone());

    Some((property_name, binding_property_name))
}

/// Try to detect and parse a signal-based input from a property initializer.
///
/// Signal-based inputs are created by calling functions like `input()` or `input.required()`
/// from `@angular/core`.
///
/// # Examples
/// ```typescript
/// readonly formGroup = input<FormGroup>();
/// readonly count = input.required<number>();
/// readonly name = input('default');
/// readonly aliasedInput = input<string>({ alias: 'myAlias' });
/// ```
///
/// Based on Angular's `input_function.ts` in the compiler-cli.
fn try_parse_signal_input<'a>(
    _allocator: &'a Allocator,
    value: &Expression<'a>,
    property_name: Atom<'a>,
) -> Option<R3InputMetadata<'a>> {
    // Check if the value is a call expression
    let call_expr = match value {
        Expression::CallExpression(call) => call,
        _ => return None,
    };

    // Determine if this is input() or input.required()
    let is_required = match &call_expr.callee {
        // input() - simple identifier call
        Expression::Identifier(id) if id.name == "input" => false,
        // input.required() - member expression call
        Expression::StaticMemberExpression(member) => {
            // Check for input.required
            if member.property.name == "required" {
                match &member.object {
                    Expression::Identifier(id) if id.name == "input" => true,
                    _ => return None,
                }
            } else if member.property.name == "input" {
                // Handle namespaced calls like `core.input()`
                if let Expression::Identifier(_) = &member.object {
                    return Some(R3InputMetadata {
                        class_property_name: property_name.clone(),
                        binding_property_name: property_name,
                        required: false,
                        is_signal: true,
                        transform_function: None,
                    });
                }
                return None;
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Parse options from arguments
    // For input(): first arg is initial value, second arg is options
    // For input.required(): first arg is options
    let options_arg_index = if is_required { 0 } else { 1 };

    let mut alias: Option<Atom<'a>> = None;

    if let Some(options_arg) = call_expr.arguments.get(options_arg_index) {
        if let Argument::ObjectExpression(obj) = options_arg {
            for prop in &obj.properties {
                if let ObjectPropertyKind::ObjectProperty(prop) = prop {
                    let Some(key_name) = get_property_key_name(&prop.key) else {
                        continue;
                    };

                    if key_name.as_str() == "alias" {
                        alias = extract_string_value(&prop.value);
                    }
                    // Note: Signal inputs don't support transform in the same way as decorator inputs.
                    // The transform is captured in the signal initializer itself.
                }
            }
        }
    }

    let binding_property_name = alias.unwrap_or_else(|| property_name.clone());

    Some(R3InputMetadata {
        class_property_name: property_name,
        binding_property_name,
        required: is_required,
        is_signal: true,
        transform_function: None, // Signal inputs don't capture transform metadata
    })
}

/// Extract @Input metadata from all properties in a class.
///
/// This function handles both:
/// - Decorator-based inputs: `@Input()`, `@Input('alias')`, `@Input({ required: true })`
/// - Signal-based inputs: `input()`, `input.required()`, `input({ alias: 'myAlias' })`
/// - Model signals: `model()`, `model.required()` (which also create outputs)
///
/// # Arguments
/// * `allocator` - The allocator for creating new nodes
/// * `class` - The class AST node to extract inputs from
///
/// # Returns
/// A vector of `R3InputMetadata` for each input found.
pub fn extract_input_metadata<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
) -> Vec<'a, R3InputMetadata<'a>> {
    let mut inputs = Vec::new_in(allocator);

    for element in &class.body.body {
        match element {
            ClassElement::PropertyDefinition(prop) => {
                // First check for @Input decorator
                if let Some(decorator) = find_decorator_by_name(&prop.decorators, "Input") {
                    let Some(class_property_name) = get_property_key_name(&prop.key) else {
                        continue;
                    };

                    let config = parse_input_config(allocator, decorator);

                    let binding_property_name =
                        config.alias.unwrap_or_else(|| class_property_name.clone());

                    inputs.push(R3InputMetadata {
                        class_property_name,
                        binding_property_name,
                        required: config.required,
                        is_signal: false,
                        transform_function: config.transform,
                    });
                }
                // Then check for signal-based input (input(), input.required(), model(), model.required())
                else if let Some(value) = &prop.value {
                    if let Some(property_name) = get_property_key_name(&prop.key) {
                        // Check for model() first since it also creates an input
                        if let Some(model_mapping) =
                            try_parse_signal_model(allocator, value, property_name)
                        {
                            inputs.push(model_mapping.input);
                        }
                        // Then check for input()
                        else if let Some(signal_input) =
                            try_parse_signal_input(allocator, value, property_name)
                        {
                            inputs.push(signal_input);
                        }
                    }
                }
            }

            ClassElement::AccessorProperty(prop) => {
                let Some(decorator) = find_decorator_by_name(&prop.decorators, "Input") else {
                    continue;
                };

                let Some(class_property_name) = get_property_key_name(&prop.key) else {
                    continue;
                };

                let config = parse_input_config(allocator, decorator);

                let binding_property_name =
                    config.alias.unwrap_or_else(|| class_property_name.clone());

                inputs.push(R3InputMetadata {
                    class_property_name,
                    binding_property_name,
                    required: config.required,
                    is_signal: false,
                    transform_function: config.transform,
                });
            }

            // Methods with @Input decorator (setter-based inputs)
            ClassElement::MethodDefinition(method) => {
                let Some(decorator) = find_decorator_by_name(&method.decorators, "Input") else {
                    continue;
                };

                let Some(class_property_name) = get_property_key_name(&method.key) else {
                    continue;
                };

                let config = parse_input_config(allocator, decorator);

                let binding_property_name =
                    config.alias.unwrap_or_else(|| class_property_name.clone());

                inputs.push(R3InputMetadata {
                    class_property_name,
                    binding_property_name,
                    required: config.required,
                    is_signal: false,
                    transform_function: config.transform,
                });
            }

            _ => {}
        }
    }

    inputs
}

// ============================================================================
// @Output Decorator Parsing
// ============================================================================

/// Parsed @Output decorator configuration.
struct OutputConfig<'a> {
    /// Alias name for the output binding (different from property name).
    alias: Option<Atom<'a>>,
}

impl<'a> Default for OutputConfig<'a> {
    fn default() -> Self {
        Self { alias: None }
    }
}

/// Parse the configuration from an @Output decorator.
///
/// Handles these variants:
/// - `@Output()` - no configuration
/// - `@Output('alias')` - string alias
fn parse_output_config<'a>(decorator: &'a Decorator<'a>) -> OutputConfig<'a> {
    let Expression::CallExpression(call) = &decorator.expression else {
        return OutputConfig::default();
    };

    let Some(first_arg) = call.arguments.first() else {
        return OutputConfig::default();
    };

    match first_arg {
        // @Output('alias')
        Argument::StringLiteral(lit) => OutputConfig { alias: Some(lit.value.clone()) },
        _ => OutputConfig::default(),
    }
}

/// Extract @Output metadata from all properties in a class.
///
/// This function handles:
/// - Decorator-based outputs: `@Output()`, `@Output('alias')`
/// - Signal-based outputs: `output()`, `output<T>()`, `output({ alias: 'myAlias' })`
/// - Model signals: `model()`, `model.required()` (which also create inputs)
///
/// # Arguments
/// * `allocator` - The allocator for creating new nodes
/// * `class` - The class AST node to extract outputs from
///
/// # Returns
/// A vector of tuples `(class_property_name, binding_property_name)` for each output found.
pub fn extract_output_metadata<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
) -> Vec<'a, (Atom<'a>, Atom<'a>)> {
    let mut outputs = Vec::new_in(allocator);

    for element in &class.body.body {
        match element {
            ClassElement::PropertyDefinition(prop) => {
                // First check for @Output decorator
                if let Some(decorator) = find_decorator_by_name(&prop.decorators, "Output") {
                    let Some(class_property_name) = get_property_key_name(&prop.key) else {
                        continue;
                    };

                    let config = parse_output_config(decorator);

                    let binding_property_name =
                        config.alias.unwrap_or_else(|| class_property_name.clone());

                    outputs.push((class_property_name, binding_property_name));
                }
                // Then check for signal-based outputs (output(), model())
                else if let Some(value) = &prop.value {
                    if let Some(property_name) = get_property_key_name(&prop.key) {
                        // Check for output() signal first
                        if let Some(output_mapping) =
                            try_parse_signal_output(value, property_name.clone())
                        {
                            outputs.push(output_mapping);
                        }
                        // Then check for model() signal which also creates an output
                        else if let Some(model_mapping) =
                            try_parse_signal_model(allocator, value, property_name)
                        {
                            outputs.push(model_mapping.output);
                        }
                    }
                }
            }

            ClassElement::AccessorProperty(prop) => {
                let Some(decorator) = find_decorator_by_name(&prop.decorators, "Output") else {
                    continue;
                };

                let Some(class_property_name) = get_property_key_name(&prop.key) else {
                    continue;
                };

                let config = parse_output_config(decorator);

                let binding_property_name =
                    config.alias.unwrap_or_else(|| class_property_name.clone());

                outputs.push((class_property_name, binding_property_name));
            }

            _ => {}
        }
    }

    outputs
}

// ============================================================================
// @ViewChild/@ViewChildren/@ContentChild/@ContentChildren Decorator Parsing
// and Signal-based Query Detection (viewChild(), viewChildren(), contentChild(), contentChildren())
// ============================================================================

/// Signal query function names.
/// These are imported from @angular/core and used to create signal-based queries.
const SIGNAL_QUERY_FNS: &[&str] = &["viewChild", "viewChildren", "contentChild", "contentChildren"];

/// Parsed query decorator configuration.
struct QueryConfig<'a> {
    /// The query predicate (type or string selectors).
    predicate: Option<QueryPredicate<'a>>,
    /// Whether this is a static query.
    is_static: bool,
    /// Expression to read from matched elements.
    read: Option<OutputExpression<'a>>,
    /// Whether to include descendants (for content queries).
    descendants: bool,
}

impl<'a> Default for QueryConfig<'a> {
    fn default() -> Self {
        Self { predicate: None, is_static: false, read: None, descendants: true }
    }
}

impl<'a> QueryConfig<'a> {
    /// Create a default QueryConfig with the correct `descendants` value for the given decorator.
    ///
    /// Per Angular's compiler: "The default value for descendants is true for every decorator
    /// except @ContentChildren."
    fn default_for(decorator_name: &str) -> Self {
        Self {
            predicate: None,
            is_static: false,
            read: None,
            // For @ContentChildren, default is false; for all others, default is true
            descendants: decorator_name != "ContentChildren",
        }
    }
}

/// Parse the configuration from a query decorator (@ViewChild, @ContentChild, etc.).
///
/// Handles these variants:
/// - `@ViewChild(MyComponent)` - type predicate
/// - `@ViewChild('refName')` - string selector
/// - `@ViewChild(MyComponent, { static: true, read: ElementRef })` - with options
///
/// The `decorator_name` is used to determine the default value for `descendants`:
/// - For `@ContentChildren`, the default is `false`
/// - For all other query decorators (`@ViewChild`, `@ViewChildren`, `@ContentChild`), the default is `true`
fn parse_query_config<'a>(
    allocator: &'a Allocator,
    decorator: &'a Decorator<'a>,
    decorator_name: &str,
) -> QueryConfig<'a> {
    let Expression::CallExpression(call) = &decorator.expression else {
        return QueryConfig::default_for(decorator_name);
    };

    let Some(first_arg) = call.arguments.first() else {
        return QueryConfig::default_for(decorator_name);
    };

    let mut config = QueryConfig::default_for(decorator_name);

    // Parse predicate from first argument
    match first_arg {
        // @ViewChild('refName') - string selector
        Argument::StringLiteral(lit) => {
            let mut selectors = Vec::new_in(allocator);
            selectors.push(lit.value.clone());
            config.predicate = Some(QueryPredicate::Selectors(selectors));
        }

        // Other expressions (identifiers, member expressions, forwardRef calls, etc.)
        _ => {
            let expr = first_arg.to_expression();
            // Unwrap forwardRef if present - Angular doesn't include forwardRef in compiled output
            let unwrapped_expr = try_unwrap_forward_ref(expr).unwrap_or(expr);
            if let Some(output_expr) = convert_oxc_expression(allocator, unwrapped_expr) {
                config.predicate = Some(QueryPredicate::Type(output_expr));
            }
        }
    }

    // Parse options from second argument if present
    if let Some(second_arg) = call.arguments.get(1) {
        if let Argument::ObjectExpression(obj) = second_arg {
            for prop in &obj.properties {
                if let ObjectPropertyKind::ObjectProperty(prop) = prop {
                    let Some(key_name) = get_property_key_name(&prop.key) else {
                        continue;
                    };

                    match key_name.as_str() {
                        "static" => {
                            config.is_static = extract_boolean_value(&prop.value).unwrap_or(false);
                        }
                        "read" => {
                            config.read = convert_oxc_expression(allocator, &prop.value);
                        }
                        "descendants" => {
                            // Use the decorator-specific default if not explicitly set
                            let default = decorator_name != "ContentChildren";
                            config.descendants =
                                extract_boolean_value(&prop.value).unwrap_or(default);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    config
}

/// Type of signal query function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignalQueryType {
    ViewChild,
    ViewChildren,
    ContentChild,
    ContentChildren,
}

impl SignalQueryType {
    /// Check if this is a single-result query (viewChild or contentChild).
    fn is_first(&self) -> bool {
        matches!(self, SignalQueryType::ViewChild | SignalQueryType::ContentChild)
    }

    /// Check if this is a view query (viewChild or viewChildren).
    fn is_view_query(&self) -> bool {
        matches!(self, SignalQueryType::ViewChild | SignalQueryType::ViewChildren)
    }

    /// Get the default descendants value for this query type.
    /// Following Angular's pattern: descendants is enabled by default except for contentChildren.
    fn default_descendants(&self) -> bool {
        !matches!(self, SignalQueryType::ContentChildren)
    }
}

/// Try to detect and parse a signal-based query from a property initializer.
///
/// Signal-based queries are created by calling functions like `viewChild()`, `viewChildren()`,
/// `contentChild()`, or `contentChildren()` from `@angular/core`.
///
/// Also handles the `.required()` variant for single-result queries:
/// - `viewChild.required()` - required view child query
/// - `contentChild.required()` - required content child query
///
/// # Examples
/// ```typescript
/// readonly content = viewChild(TemplateRef);
/// readonly requiredContent = viewChild.required(TemplateRef);
/// readonly items = viewChildren(ItemComponent);
/// readonly panel = contentChild('panel');
/// readonly requiredPanel = contentChild.required('panel');
/// readonly tabs = contentChildren(TabComponent, { descendants: true });
/// ```
fn try_parse_signal_query<'a>(
    allocator: &'a Allocator,
    value: &'a Expression<'a>,
    property_name: Atom<'a>,
) -> Option<(SignalQueryType, R3QueryMetadata<'a>)> {
    // Check if the value is a call expression
    let call_expr = match value {
        Expression::CallExpression(call) => call,
        _ => return None,
    };

    // Helper to get query type from function name
    let get_query_type = |name: &str| -> Option<SignalQueryType> {
        match name {
            "viewChild" => Some(SignalQueryType::ViewChild),
            "viewChildren" => Some(SignalQueryType::ViewChildren),
            "contentChild" => Some(SignalQueryType::ContentChild),
            "contentChildren" => Some(SignalQueryType::ContentChildren),
            _ => None,
        }
    };

    // Check if the callee is one of the signal query functions
    // Handles three patterns:
    // 1. Direct call: viewChild(), viewChildren(), contentChild(), contentChildren()
    // 2. Required call: viewChild.required(), contentChild.required()
    // 3. Namespaced call: core.viewChild(), core.viewChild.required()
    let query_type = match &call_expr.callee {
        // Pattern 1: Direct call - viewChild(), viewChildren(), etc.
        Expression::Identifier(id) => get_query_type(id.name.as_str())?,
        Expression::StaticMemberExpression(member) => {
            // Pattern 2: Required call - viewChild.required(), contentChild.required()
            if member.property.name == "required" {
                match &member.object {
                    // viewChild.required()
                    Expression::Identifier(id) => get_query_type(id.name.as_str())?,
                    // Pattern 3b: Namespaced required call - core.viewChild.required()
                    // member.object is `core.viewChild` (a StaticMemberExpression)
                    Expression::StaticMemberExpression(inner_member) => {
                        // inner_member.object should be an identifier (namespace)
                        // inner_member.property should be the query function name
                        if let Expression::Identifier(_) = &inner_member.object {
                            get_query_type(inner_member.property.name.as_str())?
                        } else {
                            return None;
                        }
                    }
                    _ => return None,
                }
            }
            // Pattern 3a: Namespaced call - core.viewChild()
            else if SIGNAL_QUERY_FNS.contains(&member.property.name.as_str()) {
                // Must be namespace.queryFn() pattern
                if let Expression::Identifier(_) = &member.object {
                    get_query_type(member.property.name.as_str())?
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
        _ => return None,
    };

    // Parse the predicate from the first argument
    let predicate_arg = call_expr.arguments.first()?;
    let predicate = match predicate_arg {
        // String selector: viewChild('myRef')
        Argument::StringLiteral(lit) => {
            let mut selectors = oxc_allocator::Vec::new_in(allocator);
            selectors.push(lit.value.clone());
            QueryPredicate::Selectors(selectors)
        }
        // Type predicate: viewChild(TemplateRef) or viewChild(forwardRef(() => MyClass))
        _ => {
            let expr = predicate_arg.to_expression();
            // Unwrap forwardRef if present - Angular doesn't include forwardRef in compiled output
            let unwrapped_expr = try_unwrap_forward_ref(expr).unwrap_or(expr);
            let output_expr = convert_oxc_expression(allocator, unwrapped_expr)?;
            QueryPredicate::Type(output_expr)
        }
    };

    // Parse options from the second argument if present
    let mut read: Option<OutputExpression<'a>> = None;
    let mut descendants = query_type.default_descendants();

    if let Some(second_arg) = call_expr.arguments.get(1) {
        if let Argument::ObjectExpression(obj) = second_arg {
            for prop in &obj.properties {
                if let ObjectPropertyKind::ObjectProperty(prop) = prop {
                    let Some(key_name) = get_property_key_name(&prop.key) else {
                        continue;
                    };

                    match key_name.as_str() {
                        "read" => {
                            read = convert_oxc_expression(allocator, &prop.value);
                        }
                        "descendants" => {
                            descendants = extract_boolean_value(&prop.value).unwrap_or(descendants);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    Some((
        query_type,
        R3QueryMetadata {
            property_name,
            first: query_type.is_first(),
            predicate,
            descendants,
            emit_distinct_changes_only: true,
            read,
            is_static: false, // Signal queries are never static
            is_signal: true,
        },
    ))
}

/// Extract @ViewChild and @ViewChildren metadata from all properties in a class.
///
/// This function handles both:
/// - Decorator-based queries: `@ViewChild(MyComponent)` / `@ViewChildren(MyComponent)`
/// - Signal-based queries: `viewChild(MyComponent)` / `viewChildren(MyComponent)`
///
/// Query decorators can appear on:
/// - Property definitions: `@ViewChild(X) prop: X;`
/// - Setter methods: `@ViewChild(X) set prop(value: X) { ... }`
/// - Getter methods: `@ViewChild(X) get prop(): X { ... }`
///
/// # Arguments
/// * `allocator` - The allocator for creating new nodes
/// * `class` - The class AST node to extract view queries from
///
/// # Returns
/// A vector of `R3QueryMetadata` for each view query found.
pub fn extract_view_queries<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
) -> Vec<'a, R3QueryMetadata<'a>> {
    // Use separate vectors to match Angular's ordering approach.
    // Angular groups queries by type, maintaining declaration order within each group:
    // 1. Signal queries first (viewChild(), viewChildren())
    // 2. @ViewChild decorator queries
    // 3. @ViewChildren decorator queries
    //
    // See: packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts
    let mut signal_queries = Vec::new_in(allocator);
    let mut view_child_queries = Vec::new_in(allocator);
    let mut view_children_queries = Vec::new_in(allocator);

    for element in &class.body.body {
        match element {
            ClassElement::PropertyDefinition(prop) => {
                // Check for signal-based view queries first (viewChild(), viewChildren())
                if let Some(value) = &prop.value {
                    if let Some(property_name) = get_property_key_name(&prop.key) {
                        if let Some((query_type, metadata)) =
                            try_parse_signal_query(allocator, value, property_name)
                        {
                            if query_type.is_view_query() {
                                signal_queries.push(metadata);
                                continue;
                            }
                        }
                    }
                }

                // Check for decorator-based queries (@ViewChild, @ViewChildren)
                if let Some(decorator) = find_decorator_by_name(&prop.decorators, "ViewChild") {
                    if let Some(property_name) = get_property_key_name(&prop.key) {
                        let config = parse_query_config(allocator, decorator, "ViewChild");
                        if let Some(predicate) = config.predicate {
                            view_child_queries.push(R3QueryMetadata {
                                property_name,
                                first: true,
                                predicate,
                                descendants: true,
                                emit_distinct_changes_only: true,
                                read: config.read,
                                is_static: config.is_static,
                                is_signal: false,
                            });
                        }
                    }
                } else if let Some(decorator) =
                    find_decorator_by_name(&prop.decorators, "ViewChildren")
                {
                    if let Some(property_name) = get_property_key_name(&prop.key) {
                        let config = parse_query_config(allocator, decorator, "ViewChildren");
                        if let Some(predicate) = config.predicate {
                            view_children_queries.push(R3QueryMetadata {
                                property_name,
                                first: false,
                                predicate,
                                descendants: true,
                                emit_distinct_changes_only: true,
                                read: config.read,
                                is_static: config.is_static,
                                is_signal: false,
                            });
                        }
                    }
                }
            }
            ClassElement::MethodDefinition(method)
                if matches!(method.kind, MethodDefinitionKind::Set | MethodDefinitionKind::Get) =>
            {
                // Check for decorator-based queries on setters/getters
                if let Some(decorator) = find_decorator_by_name(&method.decorators, "ViewChild") {
                    if let Some(property_name) = get_property_key_name(&method.key) {
                        let config = parse_query_config(allocator, decorator, "ViewChild");
                        if let Some(predicate) = config.predicate {
                            view_child_queries.push(R3QueryMetadata {
                                property_name,
                                first: true,
                                predicate,
                                descendants: true,
                                emit_distinct_changes_only: true,
                                read: config.read,
                                is_static: config.is_static,
                                is_signal: false,
                            });
                        }
                    }
                } else if let Some(decorator) =
                    find_decorator_by_name(&method.decorators, "ViewChildren")
                {
                    if let Some(property_name) = get_property_key_name(&method.key) {
                        let config = parse_query_config(allocator, decorator, "ViewChildren");
                        if let Some(predicate) = config.predicate {
                            view_children_queries.push(R3QueryMetadata {
                                property_name,
                                first: false,
                                predicate,
                                descendants: true,
                                emit_distinct_changes_only: true,
                                read: config.read,
                                is_static: config.is_static,
                                is_signal: false,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Concatenate in Angular's order: signals first, then ViewChild, then ViewChildren
    let mut result = Vec::new_in(allocator);
    result.extend(signal_queries);
    result.extend(view_child_queries);
    result.extend(view_children_queries);
    result
}

/// Extract @ContentChild and @ContentChildren metadata from all properties in a class.
///
/// This function handles both:
/// - Decorator-based queries: `@ContentChild(MyComponent)` / `@ContentChildren(MyComponent)`
/// - Signal-based queries: `contentChild(MyComponent)` / `contentChildren(MyComponent)`
///
/// Query decorators can appear on:
/// - Property definitions: `@ContentChild(X) prop: X;`
/// - Setter methods: `@ContentChildren(X) set prop(value: QueryList<X>) { ... }`
/// - Getter methods: `@ContentChild(X) get prop(): X { ... }`
///
/// # Arguments
/// * `allocator` - The allocator for creating new nodes
/// * `class` - The class AST node to extract content queries from
///
/// # Returns
/// A vector of `R3QueryMetadata` for each content query found.
pub fn extract_content_queries<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
) -> Vec<'a, R3QueryMetadata<'a>> {
    // Use separate vectors to match Angular's ordering approach.
    // Angular groups queries by type, maintaining declaration order within each group:
    // 1. Signal queries first (contentChild(), contentChildren())
    // 2. @ContentChild decorator queries
    // 3. @ContentChildren decorator queries
    //
    // See: packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts
    let mut signal_queries = Vec::new_in(allocator);
    let mut content_child_queries = Vec::new_in(allocator);
    let mut content_children_queries = Vec::new_in(allocator);

    for element in &class.body.body {
        match element {
            ClassElement::PropertyDefinition(prop) => {
                // Check for signal-based content queries first (contentChild(), contentChildren())
                if let Some(value) = &prop.value {
                    if let Some(property_name) = get_property_key_name(&prop.key) {
                        if let Some((query_type, metadata)) =
                            try_parse_signal_query(allocator, value, property_name)
                        {
                            if !query_type.is_view_query() {
                                signal_queries.push(metadata);
                                continue;
                            }
                        }
                    }
                }

                // Check for decorator-based queries (@ContentChild, @ContentChildren)
                if let Some(decorator) = find_decorator_by_name(&prop.decorators, "ContentChild") {
                    if let Some(property_name) = get_property_key_name(&prop.key) {
                        let config = parse_query_config(allocator, decorator, "ContentChild");
                        if let Some(predicate) = config.predicate {
                            content_child_queries.push(R3QueryMetadata {
                                property_name,
                                first: true,
                                predicate,
                                descendants: config.descendants,
                                emit_distinct_changes_only: true,
                                read: config.read,
                                is_static: config.is_static,
                                is_signal: false,
                            });
                        }
                    }
                } else if let Some(decorator) =
                    find_decorator_by_name(&prop.decorators, "ContentChildren")
                {
                    if let Some(property_name) = get_property_key_name(&prop.key) {
                        let config = parse_query_config(allocator, decorator, "ContentChildren");
                        if let Some(predicate) = config.predicate {
                            content_children_queries.push(R3QueryMetadata {
                                property_name,
                                first: false,
                                predicate,
                                descendants: config.descendants,
                                emit_distinct_changes_only: true,
                                read: config.read,
                                is_static: config.is_static,
                                is_signal: false,
                            });
                        }
                    }
                }
            }
            ClassElement::MethodDefinition(method)
                if matches!(method.kind, MethodDefinitionKind::Set | MethodDefinitionKind::Get) =>
            {
                // Check for decorator-based queries on setters/getters
                if let Some(decorator) = find_decorator_by_name(&method.decorators, "ContentChild")
                {
                    if let Some(property_name) = get_property_key_name(&method.key) {
                        let config = parse_query_config(allocator, decorator, "ContentChild");
                        if let Some(predicate) = config.predicate {
                            content_child_queries.push(R3QueryMetadata {
                                property_name,
                                first: true,
                                predicate,
                                descendants: config.descendants,
                                emit_distinct_changes_only: true,
                                read: config.read,
                                is_static: config.is_static,
                                is_signal: false,
                            });
                        }
                    }
                } else if let Some(decorator) =
                    find_decorator_by_name(&method.decorators, "ContentChildren")
                {
                    if let Some(property_name) = get_property_key_name(&method.key) {
                        let config = parse_query_config(allocator, decorator, "ContentChildren");
                        if let Some(predicate) = config.predicate {
                            content_children_queries.push(R3QueryMetadata {
                                property_name,
                                first: false,
                                predicate,
                                descendants: config.descendants,
                                emit_distinct_changes_only: true,
                                read: config.read,
                                is_static: config.is_static,
                                is_signal: false,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Concatenate in Angular's order: signals first, then ContentChild, then ContentChildren
    let mut result = Vec::new_in(allocator);
    result.extend(signal_queries);
    result.extend(content_child_queries);
    result.extend(content_children_queries);
    result
}

// ============================================================================
// @HostBinding/@HostListener Decorator Parsing
// ============================================================================

/// Extract @HostBinding metadata from all properties and methods in a class.
///
/// @HostBinding binds a class property/method to a host element property.
///
/// Handles:
/// - `@HostBinding('class.active')` - with binding name
/// - `@HostBinding()` - uses property name as binding name
///
/// # Arguments
/// * `allocator` - The allocator for creating new nodes
/// * `class` - The class AST node to extract host bindings from
///
/// # Returns
/// A vector of tuples `(hostPropertyName, classPropertyName)` for each host binding found.
pub fn extract_host_bindings<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
) -> Vec<'a, (Atom<'a>, Atom<'a>)> {
    let mut bindings = Vec::new_in(allocator);

    for element in &class.body.body {
        let (decorators, property_name) = match element {
            ClassElement::PropertyDefinition(prop) => {
                (&prop.decorators, get_property_key_name(&prop.key))
            }
            ClassElement::MethodDefinition(method) => {
                (&method.decorators, get_property_key_name(&method.key))
            }
            ClassElement::AccessorProperty(prop) => {
                (&prop.decorators, get_property_key_name(&prop.key))
            }
            _ => continue,
        };

        let Some(decorator) = find_decorator_by_name(decorators, "HostBinding") else {
            continue;
        };

        let Some(class_property_name) = property_name else {
            continue;
        };

        // Extract the host property name from the decorator argument
        let host_property_name =
            extract_host_binding_name(decorator).unwrap_or_else(|| class_property_name.clone());

        bindings.push((host_property_name, class_property_name));
    }

    bindings
}

/// Extract the binding name from a @HostBinding decorator.
///
/// Returns the string argument if present, None otherwise.
fn extract_host_binding_name<'a>(decorator: &'a Decorator<'a>) -> Option<Atom<'a>> {
    let Expression::CallExpression(call) = &decorator.expression else {
        return None;
    };

    let first_arg = call.arguments.first()?;

    match first_arg {
        Argument::StringLiteral(lit) => Some(lit.value.clone()),
        _ => {
            let expr = first_arg.to_expression();
            extract_string_value(expr)
        }
    }
}

/// Extract @HostListener metadata from all methods in a class.
///
/// @HostListener binds a method to a host element event.
///
/// Handles:
/// - `@HostListener('click')` - simple event
/// - `@HostListener('click', ['$event'])` - with arguments
/// - `@HostListener('window:resize', ['$event.target'])` - window event with arg
///
/// # Arguments
/// * `allocator` - The allocator for creating new nodes
/// * `class` - The class AST node to extract host listeners from
///
/// # Returns
/// A vector of tuples `(eventName, methodName, args)` for each host listener found.
pub fn extract_host_listeners<'a>(
    allocator: &'a Allocator,
    class: &'a Class<'a>,
) -> Vec<'a, (Atom<'a>, Atom<'a>, Vec<'a, Atom<'a>>)> {
    let mut listeners = Vec::new_in(allocator);

    for element in &class.body.body {
        // Handle both MethodDefinition and PropertyDefinition (for arrow function handlers)
        let (decorators, property_name) = match element {
            ClassElement::MethodDefinition(method) => {
                (&method.decorators, get_property_key_name(&method.key))
            }
            ClassElement::PropertyDefinition(prop) => {
                (&prop.decorators, get_property_key_name(&prop.key))
            }
            _ => continue,
        };

        let Some(decorator) = find_decorator_by_name(decorators, "HostListener") else {
            continue;
        };

        let Some(method_name) = property_name else {
            continue;
        };

        let (event_name, args) = parse_host_listener_config(allocator, decorator);

        let Some(event_name) = event_name else {
            continue;
        };

        listeners.push((event_name, method_name, args));
    }

    listeners
}

/// Parse the configuration from a @HostListener decorator.
///
/// Returns the event name and list of argument expressions.
fn parse_host_listener_config<'a>(
    allocator: &'a Allocator,
    decorator: &'a Decorator<'a>,
) -> (Option<Atom<'a>>, Vec<'a, Atom<'a>>) {
    let mut args = Vec::new_in(allocator);

    let Expression::CallExpression(call) = &decorator.expression else {
        return (None, args);
    };

    // First argument: event name
    let event_name = call.arguments.first().and_then(|arg| match arg {
        Argument::StringLiteral(lit) => Some(lit.value.clone()),
        _ => {
            let expr = arg.to_expression();
            extract_string_value(expr)
        }
    });

    // Second argument: array of argument strings
    if let Some(second_arg) = call.arguments.get(1) {
        if let Argument::ArrayExpression(arr) = second_arg {
            for elem in &arr.elements {
                match elem {
                    ArrayExpressionElement::StringLiteral(lit) => {
                        args.push(lit.value.clone());
                    }
                    ArrayExpressionElement::Elision(_) => {}
                    _ => {
                        let expr = elem.to_expression();
                        if let Some(s) = extract_string_value(expr) {
                            args.push(s);
                        }
                    }
                }
            }
        }
    }

    (event_name, args)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_ast::ast::{Declaration, ExportDefaultDeclarationKind, Statement};
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    /// Helper function to parse code and extract the first class.
    fn parse_class<'a>(allocator: &'a Allocator, code: &'a str) -> Option<&'a Class<'a>> {
        let source_type = SourceType::tsx();
        let parser_ret = Parser::new(allocator, code, source_type).parse();
        let program = allocator.alloc(parser_ret.program);

        for stmt in &program.body {
            let class = match stmt {
                Statement::ClassDeclaration(class) => Some(class.as_ref()),
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => Some(class.as_ref()),
                    _ => None,
                },
                Statement::ExportNamedDeclaration(export) => match &export.declaration {
                    Some(Declaration::ClassDeclaration(class)) => Some(class.as_ref()),
                    _ => None,
                },
                _ => None,
            };

            if class.is_some() {
                return class;
            }
        }

        None
    }

    // =========================================================================
    // @Input Tests
    // =========================================================================

    #[test]
    fn test_extract_simple_input() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Input() value: string;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "value");
        assert_eq!(inputs[0].binding_property_name.as_str(), "value");
        assert!(!inputs[0].required);
        assert!(!inputs[0].is_signal);
        assert!(inputs[0].transform_function.is_none());
    }

    #[test]
    fn test_extract_input_with_alias() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Input('inputAlias') value: string;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "value");
        assert_eq!(inputs[0].binding_property_name.as_str(), "inputAlias");
    }

    #[test]
    fn test_extract_input_with_config_object() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Input({ alias: 'myAlias', required: true }) value: string;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "value");
        assert_eq!(inputs[0].binding_property_name.as_str(), "myAlias");
        assert!(inputs[0].required);
    }

    #[test]
    fn test_extract_input_required_only() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Input({ required: true }) value: string;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "value");
        assert_eq!(inputs[0].binding_property_name.as_str(), "value");
        assert!(inputs[0].required);
    }

    #[test]
    fn test_extract_input_with_transform() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Input({ transform: booleanAttribute }) disabled: boolean;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "disabled");
        assert!(inputs[0].transform_function.is_some());
    }

    #[test]
    fn test_extract_multiple_inputs() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Input() name: string;
                @Input('ageAlias') age: number;
                @Input({ required: true }) id: string;
                normalProperty: string;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 3);

        assert_eq!(inputs[0].class_property_name.as_str(), "name");
        assert_eq!(inputs[0].binding_property_name.as_str(), "name");

        assert_eq!(inputs[1].class_property_name.as_str(), "age");
        assert_eq!(inputs[1].binding_property_name.as_str(), "ageAlias");

        assert_eq!(inputs[2].class_property_name.as_str(), "id");
        assert!(inputs[2].required);
    }

    #[test]
    fn test_extract_input_on_setter() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                private _value: string;

                @Input()
                set value(v: string) { this._value = v; }
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "value");
    }

    // =========================================================================
    // Signal Input Tests (input(), input.required())
    // =========================================================================

    #[test]
    fn test_signal_input_simple() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly formGroup = input<FormGroup>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "formGroup");
        assert_eq!(inputs[0].binding_property_name.as_str(), "formGroup");
        assert!(!inputs[0].required);
        assert!(inputs[0].is_signal);
        assert!(inputs[0].transform_function.is_none());
    }

    #[test]
    fn test_signal_input_required() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly count = input.required<number>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "count");
        assert_eq!(inputs[0].binding_property_name.as_str(), "count");
        assert!(inputs[0].required);
        assert!(inputs[0].is_signal);
    }

    #[test]
    fn test_signal_input_with_default_value() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly name = input('default');
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "name");
        assert_eq!(inputs[0].binding_property_name.as_str(), "name");
        assert!(!inputs[0].required);
        assert!(inputs[0].is_signal);
    }

    #[test]
    fn test_signal_input_with_alias() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly value = input<string>(undefined, { alias: 'myAlias' });
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "value");
        assert_eq!(inputs[0].binding_property_name.as_str(), "myAlias");
        assert!(!inputs[0].required);
        assert!(inputs[0].is_signal);
    }

    #[test]
    fn test_signal_input_required_with_alias() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly value = input.required<string>({ alias: 'requiredAlias' });
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "value");
        assert_eq!(inputs[0].binding_property_name.as_str(), "requiredAlias");
        assert!(inputs[0].required);
        assert!(inputs[0].is_signal);
    }

    #[test]
    fn test_mixed_decorator_and_signal_inputs() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Input() decoratorInput: string;
                readonly signalInput = input<number>();
                readonly requiredSignal = input.required<boolean>();
                normalProperty: string;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 3);

        // Decorator input
        assert_eq!(inputs[0].class_property_name.as_str(), "decoratorInput");
        assert!(!inputs[0].is_signal);

        // Signal input
        assert_eq!(inputs[1].class_property_name.as_str(), "signalInput");
        assert!(inputs[1].is_signal);
        assert!(!inputs[1].required);

        // Required signal input
        assert_eq!(inputs[2].class_property_name.as_str(), "requiredSignal");
        assert!(inputs[2].is_signal);
        assert!(inputs[2].required);
    }

    #[test]
    fn test_signal_input_not_confused_with_other_functions() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly notAnInput = someOtherFunction<string>();
                readonly alsoNotInput = myInput();
                readonly realInput = input<number>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "realInput");
        assert!(inputs[0].is_signal);
    }

    // =========================================================================
    // Model Signal Tests (model(), model.required())
    // =========================================================================

    #[test]
    fn test_signal_model_simple() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly open = model(false);
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        // Check inputs
        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "open");
        assert_eq!(inputs[0].binding_property_name.as_str(), "open");
        assert!(!inputs[0].required);
        assert!(inputs[0].is_signal);

        // Check outputs - model creates output with "Change" suffix
        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].0.as_str(), "open"); // class property name
        assert_eq!(outputs[0].1.as_str(), "openChange"); // binding name with Change suffix
    }

    #[test]
    fn test_signal_model_required() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly count = model.required<number>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        // Check inputs
        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "count");
        assert_eq!(inputs[0].binding_property_name.as_str(), "count");
        assert!(inputs[0].required);
        assert!(inputs[0].is_signal);

        // Check outputs
        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].0.as_str(), "count");
        assert_eq!(outputs[0].1.as_str(), "countChange");
    }

    #[test]
    fn test_signal_model_with_alias() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly value = model<string>(undefined, { alias: 'myAlias' });
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        // Check inputs - binding name should be alias
        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "value");
        assert_eq!(inputs[0].binding_property_name.as_str(), "myAlias");
        assert!(!inputs[0].required);
        assert!(inputs[0].is_signal);

        // Check outputs - output binding should be alias + "Change"
        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].0.as_str(), "value");
        assert_eq!(outputs[0].1.as_str(), "myAliasChange");
    }

    #[test]
    fn test_signal_model_required_with_alias() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly value = model.required<string>({ alias: 'requiredAlias' });
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        // Check inputs
        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "value");
        assert_eq!(inputs[0].binding_property_name.as_str(), "requiredAlias");
        assert!(inputs[0].required);
        assert!(inputs[0].is_signal);

        // Check outputs
        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].0.as_str(), "value");
        assert_eq!(outputs[0].1.as_str(), "requiredAliasChange");
    }

    #[test]
    fn test_mixed_model_and_input_signals() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Input() decoratorInput: string;
                readonly signalInput = input<number>();
                readonly modelSignal = model(false);
                readonly requiredModel = model.required<string>();
                normalProperty: string;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        // Check inputs - should have all 4 inputs
        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 4);

        // Decorator input
        assert_eq!(inputs[0].class_property_name.as_str(), "decoratorInput");
        assert!(!inputs[0].is_signal);

        // Signal input
        assert_eq!(inputs[1].class_property_name.as_str(), "signalInput");
        assert!(inputs[1].is_signal);
        assert!(!inputs[1].required);

        // Model signal (creates input + output)
        assert_eq!(inputs[2].class_property_name.as_str(), "modelSignal");
        assert!(inputs[2].is_signal);
        assert!(!inputs[2].required);

        // Required model signal
        assert_eq!(inputs[3].class_property_name.as_str(), "requiredModel");
        assert!(inputs[3].is_signal);
        assert!(inputs[3].required);

        // Check outputs - only model signals create outputs
        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].0.as_str(), "modelSignal");
        assert_eq!(outputs[0].1.as_str(), "modelSignalChange");
        assert_eq!(outputs[1].0.as_str(), "requiredModel");
        assert_eq!(outputs[1].1.as_str(), "requiredModelChange");
    }

    #[test]
    fn test_model_not_confused_with_other_functions() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly notAModel = someOtherFunction<string>();
                readonly alsoNotModel = myModel();
                readonly realModel = model<number>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        // Only realModel should be detected as input
        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].class_property_name.as_str(), "realModel");
        assert!(inputs[0].is_signal);

        // Only realModel should generate output
        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].0.as_str(), "realModel");
        assert_eq!(outputs[0].1.as_str(), "realModelChange");
    }

    // =========================================================================
    // @Output Tests
    // =========================================================================

    #[test]
    fn test_extract_simple_output() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Output() valueChange = new EventEmitter<string>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].0.as_str(), "valueChange");
        assert_eq!(outputs[0].1.as_str(), "valueChange");
    }

    #[test]
    fn test_extract_output_with_alias() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Output('changed') valueChange = new EventEmitter<string>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].0.as_str(), "valueChange");
        assert_eq!(outputs[0].1.as_str(), "changed");
    }

    #[test]
    fn test_extract_multiple_outputs() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Output() onClick = new EventEmitter<void>();
                @Output('hover') onHover = new EventEmitter<MouseEvent>();
                normalProperty: string;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 2);

        assert_eq!(outputs[0].0.as_str(), "onClick");
        assert_eq!(outputs[0].1.as_str(), "onClick");

        assert_eq!(outputs[1].0.as_str(), "onHover");
        assert_eq!(outputs[1].1.as_str(), "hover");
    }

    // =========================================================================
    // Signal-based output() Tests
    // =========================================================================

    #[test]
    fn test_extract_signal_output_simple() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly clicked = output();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].0.as_str(), "clicked");
        assert_eq!(outputs[0].1.as_str(), "clicked");
    }

    #[test]
    fn test_extract_signal_output_with_type() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly openChange = output<boolean>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].0.as_str(), "openChange");
        assert_eq!(outputs[0].1.as_str(), "openChange");
    }

    #[test]
    fn test_extract_signal_output_with_alias() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly valueChange = output<string>({ alias: 'changed' });
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].0.as_str(), "valueChange");
        assert_eq!(outputs[0].1.as_str(), "changed");
    }

    #[test]
    fn test_extract_multiple_signal_outputs() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly clicked = output();
                readonly openChange = output<boolean>();
                readonly valueChange = output<string>({ alias: 'changed' });
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 3);

        assert_eq!(outputs[0].0.as_str(), "clicked");
        assert_eq!(outputs[0].1.as_str(), "clicked");

        assert_eq!(outputs[1].0.as_str(), "openChange");
        assert_eq!(outputs[1].1.as_str(), "openChange");

        assert_eq!(outputs[2].0.as_str(), "valueChange");
        assert_eq!(outputs[2].1.as_str(), "changed");
    }

    #[test]
    fn test_mixed_decorator_and_signal_outputs() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Output() decoratorOutput = new EventEmitter<string>();
                readonly signalOutput = output<boolean>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 2);

        assert_eq!(outputs[0].0.as_str(), "decoratorOutput");
        assert_eq!(outputs[0].1.as_str(), "decoratorOutput");

        assert_eq!(outputs[1].0.as_str(), "signalOutput");
        assert_eq!(outputs[1].1.as_str(), "signalOutput");
    }

    // =========================================================================
    // Combined Tests
    // =========================================================================

    #[test]
    fn test_class_with_both_inputs_and_outputs() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Input() inputValue: string;
                @Output() outputEvent = new EventEmitter<string>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());

        assert_eq!(inputs.len(), 1);
        assert_eq!(outputs.len(), 1);
    }

    #[test]
    fn test_class_with_no_decorators() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                normalProperty: string;
                anotherProperty: number;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());

        assert_eq!(inputs.len(), 0);
        assert_eq!(outputs.len(), 0);
    }

    // =========================================================================
    // @ViewChild Tests
    // =========================================================================

    #[test]
    fn test_extract_view_child_with_type() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @ViewChild(ChildComponent) child: ChildComponent;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "child");
        assert!(queries[0].first);
        assert!(queries[0].descendants);
        assert!(!queries[0].is_static);
        assert!(queries[0].read.is_none());
        assert!(matches!(queries[0].predicate, QueryPredicate::Type(_)));
    }

    #[test]
    fn test_extract_view_child_with_string() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @ViewChild('myRef') child: ElementRef;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "child");
        assert!(queries[0].first);
        if let QueryPredicate::Selectors(selectors) = &queries[0].predicate {
            assert_eq!(selectors.len(), 1);
            assert_eq!(selectors[0].as_str(), "myRef");
        } else {
            panic!("Expected Selectors predicate");
        }
    }

    #[test]
    fn test_extract_view_child_with_options() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @ViewChild(ChildComponent, { static: true, read: ElementRef }) child: ElementRef;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "child");
        assert!(queries[0].first);
        assert!(queries[0].is_static);
        assert!(queries[0].read.is_some());
    }

    #[test]
    fn test_extract_view_children() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @ViewChildren(ItemComponent) items: QueryList<ItemComponent>;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "items");
        assert!(!queries[0].first); // ViewChildren returns multiple
        assert!(matches!(queries[0].predicate, QueryPredicate::Type(_)));
    }

    // =========================================================================
    // @ContentChild Tests
    // =========================================================================

    #[test]
    fn test_extract_content_child_with_type() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @ContentChild(PanelComponent) panel: PanelComponent;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_content_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "panel");
        assert!(queries[0].first);
        assert!(queries[0].descendants);
        assert!(matches!(queries[0].predicate, QueryPredicate::Type(_)));
    }

    #[test]
    fn test_extract_content_child_with_string() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @ContentChild('header') header: ElementRef;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_content_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "header");
        assert!(queries[0].first);
        if let QueryPredicate::Selectors(selectors) = &queries[0].predicate {
            assert_eq!(selectors.len(), 1);
            assert_eq!(selectors[0].as_str(), "header");
        } else {
            panic!("Expected Selectors predicate");
        }
    }

    #[test]
    fn test_extract_content_child_with_descendants_false() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @ContentChild(ItemComponent, { descendants: false }) item: ItemComponent;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_content_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "item");
        assert!(!queries[0].descendants);
    }

    #[test]
    fn test_extract_content_children() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @ContentChildren(TabComponent) tabs: QueryList<TabComponent>;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_content_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "tabs");
        assert!(!queries[0].first); // ContentChildren returns multiple
    }

    #[test]
    fn test_extract_multiple_queries() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @ViewChild('header') header: ElementRef;
                @ViewChildren(ItemComponent) items: QueryList<ItemComponent>;
                @ContentChild(PanelComponent) panel: PanelComponent;
                @ContentChildren(TabComponent) tabs: QueryList<TabComponent>;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let view_queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        let content_queries = extract_content_queries(&allocator, class.as_ref().unwrap());

        assert_eq!(view_queries.len(), 2);
        assert_eq!(content_queries.len(), 2);
    }

    // =========================================================================
    // @HostBinding Tests
    // =========================================================================

    #[test]
    fn test_extract_host_binding_with_name() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @HostBinding('class.active') isActive: boolean;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let bindings = extract_host_bindings(&allocator, class.as_ref().unwrap());
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].0.as_str(), "class.active"); // host property
        assert_eq!(bindings[0].1.as_str(), "isActive"); // class property
    }

    #[test]
    fn test_extract_host_binding_without_name() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @HostBinding() title: string;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let bindings = extract_host_bindings(&allocator, class.as_ref().unwrap());
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].0.as_str(), "title"); // Uses property name
        assert_eq!(bindings[0].1.as_str(), "title");
    }

    #[test]
    fn test_extract_host_binding_attr() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @HostBinding('attr.aria-label') ariaLabel: string;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let bindings = extract_host_bindings(&allocator, class.as_ref().unwrap());
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].0.as_str(), "attr.aria-label");
        assert_eq!(bindings[0].1.as_str(), "ariaLabel");
    }

    #[test]
    fn test_extract_host_binding_style() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @HostBinding('style.width.px') width: number;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let bindings = extract_host_bindings(&allocator, class.as_ref().unwrap());
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].0.as_str(), "style.width.px");
        assert_eq!(bindings[0].1.as_str(), "width");
    }

    #[test]
    fn test_extract_multiple_host_bindings() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @HostBinding('class.active') isActive: boolean;
                @HostBinding('class.disabled') isDisabled: boolean;
                @HostBinding('attr.role') role: string = 'button';
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let bindings = extract_host_bindings(&allocator, class.as_ref().unwrap());
        assert_eq!(bindings.len(), 3);
    }

    // =========================================================================
    // @HostListener Tests
    // =========================================================================

    #[test]
    fn test_extract_host_listener_simple() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @HostListener('click')
                onClick() {}
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let listeners = extract_host_listeners(&allocator, class.as_ref().unwrap());
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].0.as_str(), "click"); // event name
        assert_eq!(listeners[0].1.as_str(), "onClick"); // method name
        assert_eq!(listeners[0].2.len(), 0); // no args
    }

    #[test]
    fn test_extract_host_listener_with_event_arg() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @HostListener('click', ['$event'])
                onClick(event: MouseEvent) {}
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let listeners = extract_host_listeners(&allocator, class.as_ref().unwrap());
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].0.as_str(), "click");
        assert_eq!(listeners[0].1.as_str(), "onClick");
        assert_eq!(listeners[0].2.len(), 1);
        assert_eq!(listeners[0].2[0].as_str(), "$event");
    }

    #[test]
    fn test_extract_host_listener_with_multiple_args() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @HostListener('click', ['$event', '$event.target'])
                onClick(event: MouseEvent, target: Element) {}
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let listeners = extract_host_listeners(&allocator, class.as_ref().unwrap());
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].2.len(), 2);
        assert_eq!(listeners[0].2[0].as_str(), "$event");
        assert_eq!(listeners[0].2[1].as_str(), "$event.target");
    }

    #[test]
    fn test_extract_host_listener_window_event() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @HostListener('window:resize', ['$event'])
                onResize(event: Event) {}
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let listeners = extract_host_listeners(&allocator, class.as_ref().unwrap());
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].0.as_str(), "window:resize");
        assert_eq!(listeners[0].1.as_str(), "onResize");
    }

    #[test]
    fn test_extract_host_listener_document_event() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @HostListener('document:keydown.escape')
                onEscape() {}
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let listeners = extract_host_listeners(&allocator, class.as_ref().unwrap());
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].0.as_str(), "document:keydown.escape");
        assert_eq!(listeners[0].1.as_str(), "onEscape");
    }

    #[test]
    fn test_extract_multiple_host_listeners() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @HostListener('click')
                onClick() {}

                @HostListener('mouseenter')
                onMouseEnter() {}

                @HostListener('mouseleave')
                onMouseLeave() {}
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let listeners = extract_host_listeners(&allocator, class.as_ref().unwrap());
        assert_eq!(listeners.len(), 3);
    }

    #[test]
    fn test_extract_host_listener_on_property_definition() {
        // Test @HostListener on arrow function property (not method)
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @HostListener('window:beforeunload', ['$event'])
                private handleBeforeUnload = (event: BeforeUnloadEvent) => {
                    return 'Are you sure?';
                };
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let listeners = extract_host_listeners(&allocator, class.as_ref().unwrap());
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].0.as_str(), "window:beforeunload");
        assert_eq!(listeners[0].1.as_str(), "handleBeforeUnload");
        assert_eq!(listeners[0].2.len(), 1);
        assert_eq!(listeners[0].2[0].as_str(), "$event");
    }

    // =========================================================================
    // Combined Decorator Tests
    // =========================================================================

    #[test]
    fn test_class_with_all_decorator_types() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @Input() inputValue: string;
                @Output() outputEvent = new EventEmitter<string>();
                @ViewChild(ChildComponent) child: ChildComponent;
                @ContentChild('panel') panel: ElementRef;
                @HostBinding('class.active') isActive: boolean;

                @HostListener('click')
                onClick() {}
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        let view_queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        let content_queries = extract_content_queries(&allocator, class.as_ref().unwrap());
        let host_bindings = extract_host_bindings(&allocator, class.as_ref().unwrap());
        let host_listeners = extract_host_listeners(&allocator, class.as_ref().unwrap());

        assert_eq!(inputs.len(), 1);
        assert_eq!(outputs.len(), 1);
        assert_eq!(view_queries.len(), 1);
        assert_eq!(content_queries.len(), 1);
        assert_eq!(host_bindings.len(), 1);
        assert_eq!(host_listeners.len(), 1);
    }

    // =========================================================================
    // Signal-based Query Tests (viewChild, viewChildren, contentChild, contentChildren)
    // =========================================================================

    #[test]
    fn test_signal_view_child_with_type() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly content = viewChild(TemplateRef);
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "content");
        assert!(queries[0].first); // viewChild returns single
        assert!(queries[0].is_signal); // Signal-based query
        assert!(queries[0].descendants); // Default is true for viewChild
        assert!(!queries[0].is_static); // Signal queries are never static
        assert!(matches!(queries[0].predicate, QueryPredicate::Type(_)));
    }

    #[test]
    fn test_signal_view_child_with_string() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly myRef = viewChild('myRef');
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "myRef");
        assert!(queries[0].first);
        assert!(queries[0].is_signal);
        if let QueryPredicate::Selectors(selectors) = &queries[0].predicate {
            assert_eq!(selectors.len(), 1);
            assert_eq!(selectors[0].as_str(), "myRef");
        } else {
            panic!("Expected Selectors predicate");
        }
    }

    #[test]
    fn test_signal_view_child_with_options() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly child = viewChild(ChildComponent, { read: ElementRef });
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "child");
        assert!(queries[0].first);
        assert!(queries[0].is_signal);
        assert!(queries[0].read.is_some()); // read option parsed
    }

    #[test]
    fn test_signal_view_children() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly items = viewChildren(ItemComponent);
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "items");
        assert!(!queries[0].first); // viewChildren returns multiple
        assert!(queries[0].is_signal);
        assert!(matches!(queries[0].predicate, QueryPredicate::Type(_)));
    }

    #[test]
    fn test_signal_content_child() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly panel = contentChild(PanelComponent);
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_content_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "panel");
        assert!(queries[0].first); // contentChild returns single
        assert!(queries[0].is_signal);
        assert!(queries[0].descendants); // Default is true for contentChild
    }

    #[test]
    fn test_signal_content_child_with_string() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly header = contentChild('header');
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_content_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "header");
        assert!(queries[0].first);
        assert!(queries[0].is_signal);
        if let QueryPredicate::Selectors(selectors) = &queries[0].predicate {
            assert_eq!(selectors.len(), 1);
            assert_eq!(selectors[0].as_str(), "header");
        } else {
            panic!("Expected Selectors predicate");
        }
    }

    #[test]
    fn test_signal_content_children() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly tabs = contentChildren(TabComponent);
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_content_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "tabs");
        assert!(!queries[0].first); // contentChildren returns multiple
        assert!(queries[0].is_signal);
        // contentChildren defaults to false for descendants
        assert!(!queries[0].descendants);
    }

    #[test]
    fn test_signal_content_children_with_descendants_option() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly tabs = contentChildren(TabComponent, { descendants: true });
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_content_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert!(queries[0].descendants); // Explicitly set to true
    }

    #[test]
    fn test_signal_view_child_required() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly portal = viewChild.required(CdkPortal);
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "portal");
        assert!(queries[0].first); // viewChild returns single
        assert!(queries[0].is_signal);
        assert!(matches!(queries[0].predicate, QueryPredicate::Type(_)));
    }

    #[test]
    fn test_signal_view_child_required_with_string() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly myRef = viewChild.required('myRef');
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "myRef");
        assert!(queries[0].first);
        assert!(queries[0].is_signal);
        if let QueryPredicate::Selectors(selectors) = &queries[0].predicate {
            assert_eq!(selectors.len(), 1);
            assert_eq!(selectors[0].as_str(), "myRef");
        } else {
            panic!("Expected Selectors predicate");
        }
    }

    #[test]
    fn test_signal_content_child_required() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly content = contentChild.required(ContentComponent);
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_content_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].property_name.as_str(), "content");
        assert!(queries[0].first); // contentChild returns single
        assert!(queries[0].is_signal);
        assert!(queries[0].descendants); // Default is true for contentChild
    }

    #[test]
    fn test_multiple_signal_view_child_required() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly portal = viewChild.required(CdkPortal);
                readonly tabItem = viewChild.required(TabListItemDirective);
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0].property_name.as_str(), "portal");
        assert!(queries[0].is_signal);
        assert_eq!(queries[1].property_name.as_str(), "tabItem");
        assert!(queries[1].is_signal);
    }

    #[test]
    fn test_mixed_decorator_and_signal_queries() {
        // Angular's query ordering puts signal queries first, then decorator queries.
        // This matches the behavior in packages/compiler-cli/test/compliance/test_cases/signal_queries/GOLDEN_PARTIAL.js
        // (mixed_query_variants test case) where viewQueries = [signalViewChild, decoratorViewChild]
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @ViewChild(DecoratorComponent) decoratorChild: DecoratorComponent;
                readonly signalChild = viewChild(SignalComponent);
                @ContentChild('decoratorPanel') decoratorPanel: ElementRef;
                readonly signalPanel = contentChild('signalPanel');
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let view_queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        let content_queries = extract_content_queries(&allocator, class.as_ref().unwrap());

        assert_eq!(view_queries.len(), 2);
        // Signal queries come first (Angular's ordering)
        assert_eq!(view_queries[0].property_name.as_str(), "signalChild");
        assert!(view_queries[0].is_signal);
        // Then decorator queries
        assert_eq!(view_queries[1].property_name.as_str(), "decoratorChild");
        assert!(!view_queries[1].is_signal);

        assert_eq!(content_queries.len(), 2);
        // Signal queries come first (Angular's ordering)
        assert_eq!(content_queries[0].property_name.as_str(), "signalPanel");
        assert!(content_queries[0].is_signal);
        // Then decorator queries
        assert_eq!(content_queries[1].property_name.as_str(), "decoratorPanel");
        assert!(!content_queries[1].is_signal);
    }

    // =========================================================================
    // Angular Compliance Test Cases - Signal Inputs
    // Ported from: angular/packages/compiler-cli/test/compliance/test_cases/signal_inputs/
    // =========================================================================

    /// Test case: input_directive_definition
    /// From: signal_inputs/GOLDEN_PARTIAL.js
    ///
    /// Verifies that signal-based inputs (input(), input.required()) are correctly detected
    /// with proper isSignal and required flags.
    #[test]
    fn test_compliance_input_directive_definition() {
        let allocator = Allocator::default();
        // Source from: signal_inputs/input_directive_definition.ts
        let code = r#"
            class TestDir {
                counter = input(0);
                name = input.required<string>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 2);

        // counter: input(0) - optional signal input with default value
        assert_eq!(inputs[0].class_property_name.as_str(), "counter");
        assert_eq!(inputs[0].binding_property_name.as_str(), "counter");
        assert!(inputs[0].is_signal);
        assert!(!inputs[0].required);
        assert!(inputs[0].transform_function.is_none());

        // name: input.required<string>() - required signal input
        assert_eq!(inputs[1].class_property_name.as_str(), "name");
        assert_eq!(inputs[1].binding_property_name.as_str(), "name");
        assert!(inputs[1].is_signal);
        assert!(inputs[1].required);
        assert!(inputs[1].transform_function.is_none());
    }

    /// Test case: mixed_input_types
    /// From: signal_inputs/GOLDEN_PARTIAL.js
    ///
    /// Verifies that a mix of decorator @Input and signal input() are correctly handled,
    /// including aliases and transforms.
    #[test]
    fn test_compliance_mixed_input_types() {
        let allocator = Allocator::default();
        // Source from: signal_inputs/mixed_input_types.ts
        let code = r#"
            class TestDir {
                counter = input(0);
                signalWithTransform = input(false, { transform: convertToBoolean });
                signalWithTransformAndAlias = input(false, { alias: 'publicNameSignal', transform: convertToBoolean });

                @Input() decoratorInput = true;
                @Input('publicNameDecorator') decoratorInputWithAlias = true;
                @Input({ alias: 'publicNameDecorator2', transform: convertToBoolean })
                decoratorInputWithTransformAndAlias = true;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 6);

        // Signal inputs first (in declaration order)
        // counter: input(0) - optional signal
        assert_eq!(inputs[0].class_property_name.as_str(), "counter");
        assert_eq!(inputs[0].binding_property_name.as_str(), "counter");
        assert!(inputs[0].is_signal);
        assert!(!inputs[0].required);
        assert!(inputs[0].transform_function.is_none());

        // signalWithTransform: input(false, { transform: convertToBoolean })
        assert_eq!(inputs[1].class_property_name.as_str(), "signalWithTransform");
        assert_eq!(inputs[1].binding_property_name.as_str(), "signalWithTransform");
        assert!(inputs[1].is_signal);
        assert!(!inputs[1].required);
        // Note: signal input transforms are NOT captured in compiled output (per Angular spec)
        // See: transform_not_captured test case

        // signalWithTransformAndAlias: input(false, { alias: 'publicNameSignal', ... })
        assert_eq!(inputs[2].class_property_name.as_str(), "signalWithTransformAndAlias");
        assert_eq!(inputs[2].binding_property_name.as_str(), "publicNameSignal");
        assert!(inputs[2].is_signal);
        assert!(!inputs[2].required);

        // Decorator inputs (in declaration order)
        // @Input() decoratorInput
        assert_eq!(inputs[3].class_property_name.as_str(), "decoratorInput");
        assert_eq!(inputs[3].binding_property_name.as_str(), "decoratorInput");
        assert!(!inputs[3].is_signal);
        assert!(!inputs[3].required);
        assert!(inputs[3].transform_function.is_none());

        // @Input('publicNameDecorator') decoratorInputWithAlias
        assert_eq!(inputs[4].class_property_name.as_str(), "decoratorInputWithAlias");
        assert_eq!(inputs[4].binding_property_name.as_str(), "publicNameDecorator");
        assert!(!inputs[4].is_signal);
        assert!(!inputs[4].required);

        // @Input({ alias: 'publicNameDecorator2', transform: convertToBoolean })
        assert_eq!(inputs[5].class_property_name.as_str(), "decoratorInputWithTransformAndAlias");
        assert_eq!(inputs[5].binding_property_name.as_str(), "publicNameDecorator2");
        assert!(!inputs[5].is_signal);
        // Decorator inputs DO capture transform functions
        assert!(inputs[5].transform_function.is_some());
    }

    /// Test case: transform_not_captured
    /// From: signal_inputs/GOLDEN_PARTIAL.js
    ///
    /// Verifies that signal input transform functions are NOT captured in the compiled output.
    /// This is the key difference from decorator @Input transforms.
    #[test]
    fn test_compliance_signal_input_transform_not_captured() {
        let allocator = Allocator::default();
        // Source from: signal_inputs/transform_not_captured.ts
        let code = r#"
            class TestDir {
                name = input.required({ transform: convertToBoolean });
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 1);

        // Signal input transforms are NOT captured (transformFunction: null in Angular's output)
        assert_eq!(inputs[0].class_property_name.as_str(), "name");
        assert!(inputs[0].is_signal);
        assert!(inputs[0].required);
        // Key assertion: transform is NOT captured for signal inputs
        assert!(inputs[0].transform_function.is_none());
    }

    /// Test case: complex_transform_functions
    /// From: signal_inputs/GOLDEN_PARTIAL.js
    ///
    /// Verifies that complex transform functions (arrow functions, generics) work with signal inputs.
    /// These patterns were NOT supported with @Input decorators.
    #[test]
    fn test_compliance_complex_transform_functions() {
        let allocator = Allocator::default();
        // Source from: signal_inputs/complex_transform_functions.ts
        let code = r#"
            class TestDir {
                name = input.required<boolean, string|boolean>({
                    transform: (v) => v === true || v !== '',
                });
                name2 = input.required<boolean, string|boolean>({ transform: toBoolean });

                genericTransform = input.required({ transform: complexTransform(1) });
                genericTransform2 = input.required({ transform: complexTransform(null) });
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(inputs.len(), 4);

        // All should be required signal inputs with NO transform captured
        for (i, name) in
            ["name", "name2", "genericTransform", "genericTransform2"].iter().enumerate()
        {
            assert_eq!(inputs[i].class_property_name.as_str(), *name);
            assert!(inputs[i].is_signal, "Input {} should be signal", name);
            assert!(inputs[i].required, "Input {} should be required", name);
            // Transform is NOT captured for signal inputs
            assert!(
                inputs[i].transform_function.is_none(),
                "Input {} should NOT have transform captured",
                name
            );
        }
    }

    // =========================================================================
    // Angular Compliance Test Cases - Signal Queries
    // Ported from: angular/packages/compiler-cli/test/compliance/test_cases/signal_queries/
    // =========================================================================

    /// Test case: query_in_directive
    /// From: signal_queries/GOLDEN_PARTIAL.js
    ///
    /// Verifies signal-based queries (viewChild, viewChildren, contentChild, contentChildren)
    /// with various predicates (string selectors, types, forwardRef).
    #[test]
    fn test_compliance_query_in_directive() {
        let allocator = Allocator::default();
        // Source from: signal_queries/query_in_directive.ts (simplified)
        let code = r#"
            class TestDir {
                query1 = viewChild('locatorA');
                query2 = viewChildren('locatorB');
                query3 = contentChild('locatorC');
                query4 = contentChildren('locatorD');
                query5 = viewChild(SomeToken);
                query6 = viewChildren(SomeToken);
                query7 = viewChild('locatorE', { read: SomeToken });
                query8 = contentChildren('locatorF, locatorG', { descendants: true });
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let view_queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        let content_queries = extract_content_queries(&allocator, class.as_ref().unwrap());

        // View queries: query1, query2, query5, query6, query7
        assert_eq!(view_queries.len(), 5);

        // query1: viewChild('locatorA') - single, signal, string selector
        assert_eq!(view_queries[0].property_name.as_str(), "query1");
        assert!(view_queries[0].first);
        assert!(view_queries[0].is_signal);
        assert!(view_queries[0].descendants);
        if let QueryPredicate::Selectors(selectors) = &view_queries[0].predicate {
            assert_eq!(selectors[0].as_str(), "locatorA");
        } else {
            panic!("Expected Selectors predicate for query1");
        }

        // query2: viewChildren('locatorB') - multiple, signal
        assert_eq!(view_queries[1].property_name.as_str(), "query2");
        assert!(!view_queries[1].first); // viewChildren = not first
        assert!(view_queries[1].is_signal);

        // query5: viewChild(SomeToken) - single, signal, type predicate
        assert_eq!(view_queries[2].property_name.as_str(), "query5");
        assert!(view_queries[2].first);
        assert!(view_queries[2].is_signal);
        assert!(matches!(view_queries[2].predicate, QueryPredicate::Type(_)));

        // query6: viewChildren(SomeToken) - multiple, signal, type predicate
        assert_eq!(view_queries[3].property_name.as_str(), "query6");
        assert!(!view_queries[3].first);
        assert!(view_queries[3].is_signal);

        // query7: viewChild('locatorE', { read: SomeToken }) - with read option
        assert_eq!(view_queries[4].property_name.as_str(), "query7");
        assert!(view_queries[4].first);
        assert!(view_queries[4].is_signal);
        assert!(view_queries[4].read.is_some());

        // Content queries: query3, query4, query8
        assert_eq!(content_queries.len(), 3);

        // query3: contentChild('locatorC') - single, signal
        assert_eq!(content_queries[0].property_name.as_str(), "query3");
        assert!(content_queries[0].first);
        assert!(content_queries[0].is_signal);
        assert!(content_queries[0].descendants); // contentChild defaults to true

        // query4: contentChildren('locatorD') - multiple, signal
        assert_eq!(content_queries[1].property_name.as_str(), "query4");
        assert!(!content_queries[1].first);
        assert!(content_queries[1].is_signal);
        // contentChildren defaults to descendants: false
        assert!(!content_queries[1].descendants);

        // query8: contentChildren('locatorF, locatorG', { descendants: true })
        assert_eq!(content_queries[2].property_name.as_str(), "query8");
        assert!(!content_queries[2].first);
        assert!(content_queries[2].is_signal);
        assert!(content_queries[2].descendants); // explicitly set to true
    }

    /// Test case: mixed_query_variants
    /// From: signal_queries/GOLDEN_PARTIAL.js (lines 81-103)
    ///
    /// Verifies that signal queries come FIRST, then decorator queries.
    /// This is the key ordering behavior.
    #[test]
    fn test_compliance_mixed_query_variants() {
        let allocator = Allocator::default();
        // Source from: signal_queries/mixed_query_variants.ts
        let code = r#"
            class TestDir {
                @ViewChild('locator1') decoratorViewChild;
                signalViewChild = viewChild('locator1');
                @ContentChild('locator2') decoratorContentChild;
                signalContentChild = contentChild('locator2');
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let view_queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        let content_queries = extract_content_queries(&allocator, class.as_ref().unwrap());

        // Angular ordering: signal queries FIRST, then decorator queries
        // viewQueries: [signalViewChild, decoratorViewChild]
        assert_eq!(view_queries.len(), 2);
        assert_eq!(view_queries[0].property_name.as_str(), "signalViewChild");
        assert!(view_queries[0].is_signal);
        assert_eq!(view_queries[1].property_name.as_str(), "decoratorViewChild");
        assert!(!view_queries[1].is_signal);

        // queries (content): [signalContentChild, decoratorContentChild]
        assert_eq!(content_queries.len(), 2);
        assert_eq!(content_queries[0].property_name.as_str(), "signalContentChild");
        assert!(content_queries[0].is_signal);
        assert_eq!(content_queries[1].property_name.as_str(), "decoratorContentChild");
        assert!(!content_queries[1].is_signal);
    }

    // =========================================================================
    // Angular Compliance Test Cases - Output Function
    // Ported from: angular/packages/compiler-cli/test/compliance/test_cases/output_function/
    // =========================================================================

    /// Test case: output_in_directive
    /// From: output_function/GOLDEN_PARTIAL.js
    ///
    /// Verifies signal-based outputs with output() function and aliases.
    #[test]
    fn test_compliance_output_in_directive() {
        let allocator = Allocator::default();
        // Source from: output_function/output_in_directive.ts
        let code = r#"
            class TestDir {
                a = output();
                b = output({});
                c = output({ alias: 'cPublic' });
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 3);

        // a: output() - no alias
        assert_eq!(outputs[0].0.as_str(), "a");
        assert_eq!(outputs[0].1.as_str(), "a");

        // b: output({}) - empty options
        assert_eq!(outputs[1].0.as_str(), "b");
        assert_eq!(outputs[1].1.as_str(), "b");

        // c: output({ alias: 'cPublic' }) - with alias
        assert_eq!(outputs[2].0.as_str(), "c");
        assert_eq!(outputs[2].1.as_str(), "cPublic");
    }

    /// Test case: mixed_variants
    /// From: output_function/GOLDEN_PARTIAL.js
    ///
    /// Verifies mix of signal output() and decorator @Output.
    #[test]
    fn test_compliance_mixed_output_variants() {
        let allocator = Allocator::default();
        // Source from: output_function/mixed_variants.ts
        let code = r#"
            class TestDir {
                click1 = output();
                click2 = output();
                _bla = output({ alias: 'decoratorPublicName' });

                @Output() clickDecorator1 = new EventEmitter();
                @Output() clickDecorator2 = new EventEmitter();
                @Output('decoratorPublicName') _blaDecorator = new EventEmitter();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());
        assert_eq!(outputs.len(), 6);

        // Signal outputs first (in declaration order)
        assert_eq!(outputs[0].0.as_str(), "click1");
        assert_eq!(outputs[0].1.as_str(), "click1");

        assert_eq!(outputs[1].0.as_str(), "click2");
        assert_eq!(outputs[1].1.as_str(), "click2");

        // Signal output with alias
        assert_eq!(outputs[2].0.as_str(), "_bla");
        assert_eq!(outputs[2].1.as_str(), "decoratorPublicName");

        // Decorator outputs
        assert_eq!(outputs[3].0.as_str(), "clickDecorator1");
        assert_eq!(outputs[3].1.as_str(), "clickDecorator1");

        assert_eq!(outputs[4].0.as_str(), "clickDecorator2");
        assert_eq!(outputs[4].1.as_str(), "clickDecorator2");

        // Decorator output with alias
        assert_eq!(outputs[5].0.as_str(), "_blaDecorator");
        assert_eq!(outputs[5].1.as_str(), "decoratorPublicName");
    }

    // =========================================================================
    // Angular Compliance Test Cases - Model Inputs
    // Ported from: angular/packages/compiler-cli/test/compliance/test_cases/model_inputs/
    // =========================================================================

    /// Test case: model_directive_definition
    /// From: model_inputs/GOLDEN_PARTIAL.js
    ///
    /// Verifies model() creates both input AND output bindings.
    #[test]
    fn test_compliance_model_directive_definition() {
        let allocator = Allocator::default();
        // Source from: model_inputs/model_directive_definition.ts
        let code = r#"
            class TestDir {
                counter = model(0);
                name = model.required<string>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());

        // Model creates inputs
        assert_eq!(inputs.len(), 2);

        // counter: model(0) - optional signal input
        assert_eq!(inputs[0].class_property_name.as_str(), "counter");
        assert_eq!(inputs[0].binding_property_name.as_str(), "counter");
        assert!(inputs[0].is_signal);
        assert!(!inputs[0].required);

        // name: model.required<string>() - required signal input
        assert_eq!(inputs[1].class_property_name.as_str(), "name");
        assert_eq!(inputs[1].binding_property_name.as_str(), "name");
        assert!(inputs[1].is_signal);
        assert!(inputs[1].required);

        // Model creates outputs with "Change" suffix
        assert_eq!(outputs.len(), 2);

        // counter -> counterChange
        assert_eq!(outputs[0].0.as_str(), "counter");
        assert_eq!(outputs[0].1.as_str(), "counterChange");

        // name -> nameChange
        assert_eq!(outputs[1].0.as_str(), "name");
        assert_eq!(outputs[1].1.as_str(), "nameChange");
    }

    /// Test case: mixed_model_types
    /// From: model_inputs/GOLDEN_PARTIAL.js
    ///
    /// Verifies mix of model() signals with decorator @Input/@Output.
    #[test]
    fn test_compliance_mixed_model_types() {
        let allocator = Allocator::default();
        // Source from: model_inputs/mixed_model_types.ts
        let code = r#"
            class TestDir {
                counter = model(0);
                modelWithAlias = model(false, { alias: 'alias' });

                @Input() decoratorInput = true;
                @Input('publicNameDecorator') decoratorInputWithAlias = true;
                @Output() decoratorOutput = new EventEmitter();
                @Output('aliasDecoratorOutputWithAlias') decoratorOutputWithAlias = new EventEmitter();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let inputs = extract_input_metadata(&allocator, class.as_ref().unwrap());
        let outputs = extract_output_metadata(&allocator, class.as_ref().unwrap());

        // Inputs: 2 from model + 2 from @Input = 4
        assert_eq!(inputs.len(), 4);

        // Model inputs (signal-based)
        assert_eq!(inputs[0].class_property_name.as_str(), "counter");
        assert_eq!(inputs[0].binding_property_name.as_str(), "counter");
        assert!(inputs[0].is_signal);

        // Model with alias
        assert_eq!(inputs[1].class_property_name.as_str(), "modelWithAlias");
        assert_eq!(inputs[1].binding_property_name.as_str(), "alias");
        assert!(inputs[1].is_signal);

        // Decorator inputs
        assert_eq!(inputs[2].class_property_name.as_str(), "decoratorInput");
        assert_eq!(inputs[2].binding_property_name.as_str(), "decoratorInput");
        assert!(!inputs[2].is_signal);

        assert_eq!(inputs[3].class_property_name.as_str(), "decoratorInputWithAlias");
        assert_eq!(inputs[3].binding_property_name.as_str(), "publicNameDecorator");
        assert!(!inputs[3].is_signal);

        // Outputs: 2 from model + 2 from @Output = 4
        assert_eq!(outputs.len(), 4);

        // Model outputs (with Change suffix)
        assert_eq!(outputs[0].0.as_str(), "counter");
        assert_eq!(outputs[0].1.as_str(), "counterChange");

        // Model with alias -> aliasChange
        assert_eq!(outputs[1].0.as_str(), "modelWithAlias");
        assert_eq!(outputs[1].1.as_str(), "aliasChange");

        // Decorator outputs
        assert_eq!(outputs[2].0.as_str(), "decoratorOutput");
        assert_eq!(outputs[2].1.as_str(), "decoratorOutput");

        assert_eq!(outputs[3].0.as_str(), "decoratorOutputWithAlias");
        assert_eq!(outputs[3].1.as_str(), "aliasDecoratorOutputWithAlias");
    }

    /// Test: viewChild.required() - required signal query
    /// From: Angular's signal query API
    #[test]
    fn test_compliance_required_signal_query() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                readonly content = viewChild.required(TemplateRef);
                readonly header = contentChild.required('header');
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let view_queries = extract_view_queries(&allocator, class.as_ref().unwrap());
        let content_queries = extract_content_queries(&allocator, class.as_ref().unwrap());

        // viewChild.required() creates a required signal query
        assert_eq!(view_queries.len(), 1);
        assert_eq!(view_queries[0].property_name.as_str(), "content");
        assert!(view_queries[0].first);
        assert!(view_queries[0].is_signal);
        assert!(matches!(view_queries[0].predicate, QueryPredicate::Type(_)));

        // contentChild.required() creates a required signal content query
        assert_eq!(content_queries.len(), 1);
        assert_eq!(content_queries[0].property_name.as_str(), "header");
        assert!(content_queries[0].first);
        assert!(content_queries[0].is_signal);
    }
}
