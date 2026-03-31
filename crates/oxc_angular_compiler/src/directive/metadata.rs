//! Directive metadata structures.
//!
//! Ported from Angular's `render3/view/api.ts`.

use oxc_allocator::{Allocator, Vec};
use oxc_ast::ast::Class;
use oxc_span::Ident;

use crate::factory::R3DependencyMetadata;
use crate::output::ast::OutputExpression;

/// Metadata for an individual input on a directive.
///
/// Corresponds to Angular's `R3InputMetadata` interface.
#[derive(Debug)]
pub struct R3InputMetadata<'a> {
    /// The property name on the class.
    pub class_property_name: Ident<'a>,

    /// The binding property name (can differ from class property name).
    pub binding_property_name: Ident<'a>,

    /// Whether this input is required.
    pub required: bool,

    /// Whether this is a signal-based input.
    pub is_signal: bool,

    /// Transform function for the input.
    /// Null if there is no transform, or if this is a signal input.
    pub transform_function: Option<OutputExpression<'a>>,
}

impl<'a> R3InputMetadata<'a> {
    /// Create a simple input with matching class and binding names.
    pub fn simple(name: Ident<'a>) -> Self {
        Self {
            class_property_name: name.clone(),
            binding_property_name: name,
            required: false,
            is_signal: false,
            transform_function: None,
        }
    }

    /// Create a required input.
    pub fn required(name: Ident<'a>) -> Self {
        Self {
            class_property_name: name.clone(),
            binding_property_name: name,
            required: true,
            is_signal: false,
            transform_function: None,
        }
    }

    /// Create a signal-based input.
    pub fn signal(name: Ident<'a>) -> Self {
        Self {
            class_property_name: name.clone(),
            binding_property_name: name,
            required: false,
            is_signal: true,
            transform_function: None,
        }
    }
}

/// Metadata for a query (view or content).
///
/// Corresponds to Angular's `R3QueryMetadata` interface.
#[derive(Debug)]
pub struct R3QueryMetadata<'a> {
    /// Name of the property on the class to update with query results.
    pub property_name: Ident<'a>,

    /// Whether to read only the first matching result.
    pub first: bool,

    /// The query predicate (type expression or string selectors).
    pub predicate: QueryPredicate<'a>,

    /// Whether to include only direct children or all descendants.
    pub descendants: bool,

    /// If the QueryList should fire change event only if actual change occurred.
    pub emit_distinct_changes_only: bool,

    /// An expression representing a type to read from each matched node.
    pub read: Option<OutputExpression<'a>>,

    /// Whether this query should collect only static results.
    pub is_static: bool,

    /// Whether the query is signal-based.
    pub is_signal: bool,
}

/// Query predicate type.
#[derive(Debug)]
pub enum QueryPredicate<'a> {
    /// Type or InjectionToken expression.
    Type(OutputExpression<'a>),

    /// String selectors.
    Selectors(Vec<'a, Ident<'a>>),
}

impl<'a> R3QueryMetadata<'a> {
    /// Create a new query metadata with defaults.
    pub fn new(allocator: &'a Allocator, property_name: Ident<'a>) -> Self {
        Self {
            property_name,
            first: false,
            predicate: QueryPredicate::Selectors(Vec::new_in(allocator)),
            descendants: true,
            emit_distinct_changes_only: true,
            read: None,
            is_static: false,
            is_signal: false,
        }
    }
}

/// Host metadata for bindings and listeners.
///
/// Corresponds to Angular's `R3HostMetadata` interface.
#[derive(Debug)]
pub struct R3HostMetadata<'a> {
    /// A mapping of attribute binding keys to expressions.
    pub attributes: Vec<'a, (Ident<'a>, OutputExpression<'a>)>,

    /// A mapping of event binding keys to unparsed expressions.
    pub listeners: Vec<'a, (Ident<'a>, Ident<'a>)>,

    /// A mapping of property binding keys to unparsed expressions.
    pub properties: Vec<'a, (Ident<'a>, Ident<'a>)>,

    /// Special style attribute value.
    pub style_attr: Option<Ident<'a>>,

    /// Special class attribute value.
    pub class_attr: Option<Ident<'a>>,
}

impl<'a> R3HostMetadata<'a> {
    /// Create a new empty host metadata.
    pub fn new(allocator: &'a Allocator) -> Self {
        Self {
            attributes: Vec::new_in(allocator),
            listeners: Vec::new_in(allocator),
            properties: Vec::new_in(allocator),
            style_attr: None,
            class_attr: None,
        }
    }

    /// Check if there are any host bindings.
    pub fn has_bindings(&self) -> bool {
        !self.attributes.is_empty()
            || !self.listeners.is_empty()
            || !self.properties.is_empty()
            || self.style_attr.is_some()
            || self.class_attr.is_some()
    }
}

/// Host directive metadata.
///
/// Corresponds to Angular's `R3HostDirectiveMetadata` interface.
#[derive(Debug)]
pub struct R3HostDirectiveMetadata<'a> {
    /// An expression representing the host directive class.
    pub directive: OutputExpression<'a>,

    /// Whether the expression is a forward reference.
    pub is_forward_reference: bool,

    /// Inputs from the host directive that will be exposed on the host.
    /// Key: public name, Value: internal name
    pub inputs: Vec<'a, (Ident<'a>, Ident<'a>)>,

    /// Outputs from the host directive that will be exposed on the host.
    /// Key: public name, Value: internal name
    pub outputs: Vec<'a, (Ident<'a>, Ident<'a>)>,
}

/// Metadata needed to compile a directive.
///
/// Corresponds to Angular's `R3DirectiveMetadata` interface.
#[derive(Debug)]
pub struct R3DirectiveMetadata<'a> {
    /// Name of the directive type.
    pub name: Ident<'a>,

    /// An expression representing a reference to the directive itself.
    pub r#type: OutputExpression<'a>,

    /// Number of generic type parameters of the type itself.
    pub type_argument_count: u32,

    /// Dependencies of the directive's constructor.
    pub deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,

    /// Unparsed selector of the directive, or None if there was no selector.
    pub selector: Option<Ident<'a>>,

    /// Content queries made by the directive.
    pub queries: Vec<'a, R3QueryMetadata<'a>>,

    /// View queries made by the directive.
    pub view_queries: Vec<'a, R3QueryMetadata<'a>>,

    /// Host metadata (bindings, listeners, attributes).
    pub host: R3HostMetadata<'a>,

    /// Whether the directive uses NgOnChanges.
    pub uses_on_changes: bool,

    /// Inputs of the directive.
    pub inputs: Vec<'a, R3InputMetadata<'a>>,

    /// Outputs of the directive (class property name -> binding property name).
    pub outputs: Vec<'a, (Ident<'a>, Ident<'a>)>,

    /// Whether or not the directive inherits from another class.
    pub uses_inheritance: bool,

    /// Export names for template references.
    pub export_as: Vec<'a, Ident<'a>>,

    /// The list of providers defined in the directive.
    pub providers: Option<OutputExpression<'a>>,

    /// Whether this is a standalone directive.
    pub is_standalone: bool,

    /// Whether this is a signal-based directive.
    pub is_signal: bool,

    /// Additional directives applied to the directive host.
    pub host_directives: Vec<'a, R3HostDirectiveMetadata<'a>>,
}

impl<'a> R3DirectiveMetadata<'a> {
    /// Check if this directive has any features that need to be emitted.
    pub fn has_features(&self) -> bool {
        self.providers.is_some()
            || !self.host_directives.is_empty()
            || self.uses_inheritance
            || self.uses_on_changes
    }
}

/// Builder for R3DirectiveMetadata.
pub struct R3DirectiveMetadataBuilder<'a> {
    name: Option<Ident<'a>>,
    r#type: Option<OutputExpression<'a>>,
    type_argument_count: u32,
    deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,
    selector: Option<Ident<'a>>,
    queries: Vec<'a, R3QueryMetadata<'a>>,
    view_queries: Vec<'a, R3QueryMetadata<'a>>,
    host: R3HostMetadata<'a>,
    uses_on_changes: bool,
    inputs: Vec<'a, R3InputMetadata<'a>>,
    outputs: Vec<'a, (Ident<'a>, Ident<'a>)>,
    uses_inheritance: bool,
    export_as: Vec<'a, Ident<'a>>,
    providers: Option<OutputExpression<'a>>,
    is_standalone: bool,
    is_signal: bool,
    host_directives: Vec<'a, R3HostDirectiveMetadata<'a>>,
}

impl<'a> R3DirectiveMetadataBuilder<'a> {
    /// Create a new builder.
    pub fn new(allocator: &'a Allocator) -> Self {
        Self {
            name: None,
            r#type: None,
            type_argument_count: 0,
            deps: None,
            selector: None,
            queries: Vec::new_in(allocator),
            view_queries: Vec::new_in(allocator),
            host: R3HostMetadata::new(allocator),
            uses_on_changes: false,
            inputs: Vec::new_in(allocator),
            outputs: Vec::new_in(allocator),
            uses_inheritance: false,
            export_as: Vec::new_in(allocator),
            providers: None,
            is_standalone: true, // Default to standalone in modern Angular
            is_signal: false,
            host_directives: Vec::new_in(allocator),
        }
    }

    /// Set the directive name.
    pub fn name(mut self, name: Ident<'a>) -> Self {
        self.name = Some(name);
        self
    }

    /// Set the directive type expression.
    pub fn r#type(mut self, type_expr: OutputExpression<'a>) -> Self {
        self.r#type = Some(type_expr);
        self
    }

    /// Set the type argument count.
    pub fn type_argument_count(mut self, count: u32) -> Self {
        self.type_argument_count = count;
        self
    }

    /// Set the constructor dependencies.
    pub fn deps(mut self, deps: Vec<'a, R3DependencyMetadata<'a>>) -> Self {
        self.deps = Some(deps);
        self
    }

    /// Set the selector.
    pub fn selector(mut self, selector: Ident<'a>) -> Self {
        self.selector = Some(selector);
        self
    }

    /// Add a content query.
    pub fn add_query(mut self, query: R3QueryMetadata<'a>) -> Self {
        self.queries.push(query);
        self
    }

    /// Add a view query.
    pub fn add_view_query(mut self, query: R3QueryMetadata<'a>) -> Self {
        self.view_queries.push(query);
        self
    }

    /// Set the host metadata.
    pub fn host(mut self, host: R3HostMetadata<'a>) -> Self {
        self.host = host;
        self
    }

    /// Set whether the directive uses ngOnChanges.
    pub fn uses_on_changes(mut self, uses: bool) -> Self {
        self.uses_on_changes = uses;
        self
    }

    /// Add an input.
    pub fn add_input(mut self, input: R3InputMetadata<'a>) -> Self {
        self.inputs.push(input);
        self
    }

    /// Add an output.
    pub fn add_output(mut self, class_name: Ident<'a>, binding_name: Ident<'a>) -> Self {
        self.outputs.push((class_name, binding_name));
        self
    }

    /// Set whether the directive uses inheritance.
    pub fn uses_inheritance(mut self, uses: bool) -> Self {
        self.uses_inheritance = uses;
        self
    }

    /// Add an export name.
    pub fn add_export_as(mut self, export_name: Ident<'a>) -> Self {
        self.export_as.push(export_name);
        self
    }

    /// Set the providers expression.
    pub fn providers(mut self, providers: OutputExpression<'a>) -> Self {
        self.providers = Some(providers);
        self
    }

    /// Set whether the directive is standalone.
    pub fn is_standalone(mut self, standalone: bool) -> Self {
        self.is_standalone = standalone;
        self
    }

    /// Set whether the directive is signal-based.
    pub fn is_signal(mut self, signal: bool) -> Self {
        self.is_signal = signal;
        self
    }

    /// Add a host directive.
    pub fn add_host_directive(mut self, host_directive: R3HostDirectiveMetadata<'a>) -> Self {
        self.host_directives.push(host_directive);
        self
    }

    /// Extract directive metadata from class property and method decorators.
    ///
    /// This includes @Input, @Output, @ViewChild, @ViewChildren, @ContentChild,
    /// @ContentChildren, @HostBinding, and @HostListener decorators.
    ///
    /// # Arguments
    /// * `allocator` - The allocator for creating new nodes
    /// * `class` - The class AST node to extract metadata from
    ///
    /// # Returns
    /// The builder with all extracted metadata added.
    pub fn extract_from_class(mut self, allocator: &'a Allocator, class: &'a Class<'a>) -> Self {
        // Extract inputs from @Input decorators
        let inputs = super::property_decorators::extract_input_metadata(allocator, class);
        for input in inputs {
            self = self.add_input(input);
        }

        // Extract outputs from @Output decorators
        let outputs = super::property_decorators::extract_output_metadata(allocator, class);
        for (class_name, binding_name) in outputs {
            self = self.add_output(class_name, binding_name);
        }

        // Extract view queries from @ViewChild/@ViewChildren
        let view_queries = super::property_decorators::extract_view_queries(allocator, class);
        for query in view_queries {
            self = self.add_view_query(query);
        }

        // Extract content queries from @ContentChild/@ContentChildren
        let content_queries = super::property_decorators::extract_content_queries(allocator, class);
        for query in content_queries {
            self = self.add_query(query);
        }

        // Extract host bindings from @HostBinding
        // Wrap with brackets: "class.active" -> "[class.active]"
        let host_bindings = super::property_decorators::extract_host_bindings(allocator, class);
        for (host_prop, class_prop) in host_bindings {
            // Add to host.properties with wrapped key
            let wrapped_key = Ident::from(allocator.alloc_str(&format!("[{}]", host_prop.as_str())));
            self.host.properties.push((wrapped_key, class_prop));
        }

        // Extract host listeners from @HostListener
        // Wrap event name with parentheses and build method expression with args
        // Reference: Angular's shared.ts:713 - `bindings.listeners[eventName] = \`${member.name}(${args.join(',')})\``
        let host_listeners = super::property_decorators::extract_host_listeners(allocator, class);
        for (event_name, method_name, args) in host_listeners {
            // Wrap event name: "click" -> "(click)"
            let wrapped_key =
                Ident::from(allocator.alloc_str(&format!("({})", event_name.as_str())));

            // Build method expression with args: "handleClick" + ["$event"] -> "handleClick($event)"
            let method_expr = if args.is_empty() {
                Ident::from(allocator.alloc_str(&format!("{}()", method_name.as_str())))
            } else {
                let args_str: String =
                    args.iter().map(|a| a.as_str()).collect::<std::vec::Vec<_>>().join(",");
                Ident::from(allocator.alloc_str(&format!("{}({})", method_name.as_str(), args_str)))
            };

            // Add to host.listeners
            self.host.listeners.push((wrapped_key, method_expr));
        }

        self
    }

    /// Build the metadata.
    ///
    /// Returns None if required fields (name, type) are missing.
    pub fn build(self) -> Option<R3DirectiveMetadata<'a>> {
        let name = self.name?;
        let r#type = self.r#type?;

        Some(R3DirectiveMetadata {
            name,
            r#type,
            type_argument_count: self.type_argument_count,
            deps: self.deps,
            selector: self.selector,
            queries: self.queries,
            view_queries: self.view_queries,
            host: self.host,
            uses_on_changes: self.uses_on_changes,
            inputs: self.inputs,
            outputs: self.outputs,
            uses_inheritance: self.uses_inheritance,
            export_as: self.export_as,
            providers: self.providers,
            is_standalone: self.is_standalone,
            is_signal: self.is_signal,
            host_directives: self.host_directives,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::ast::OutputAstBuilder;
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
    // R3DirectiveMetadataBuilder Tests
    // =========================================================================

    #[test]
    fn test_builder_basic() {
        let allocator = Allocator::default();
        let builder = R3DirectiveMetadataBuilder::new(&allocator)
            .name(Ident::from("TestDirective"))
            .r#type(OutputAstBuilder::variable(&allocator, Ident::from("TestDirective")))
            .selector(Ident::from("[test]"));

        let metadata = builder.build();
        assert!(metadata.is_some());

        let metadata = metadata.unwrap();
        assert_eq!(metadata.name.as_str(), "TestDirective");
        assert_eq!(metadata.selector.as_ref().map(|s| s.as_str()), Some("[test]"));
        assert!(metadata.is_standalone); // Default is true
    }

    #[test]
    fn test_builder_missing_name_returns_none() {
        let allocator = Allocator::default();
        let builder = R3DirectiveMetadataBuilder::new(&allocator)
            .r#type(OutputAstBuilder::variable(&allocator, Ident::from("TestDirective")));

        let metadata = builder.build();
        assert!(metadata.is_none());
    }

    #[test]
    fn test_builder_missing_type_returns_none() {
        let allocator = Allocator::default();
        let builder = R3DirectiveMetadataBuilder::new(&allocator).name(Ident::from("TestDirective"));

        let metadata = builder.build();
        assert!(metadata.is_none());
    }

    // =========================================================================
    // extract_from_class Integration Tests
    // =========================================================================

    #[test]
    fn test_extract_from_class_with_inputs() {
        let allocator = Allocator::default();
        let code = r#"
            class TestDirective {
                @Input() name: string;
                @Input('aliasedValue') value: number;
                @Input({ required: true }) requiredInput: string;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let builder = R3DirectiveMetadataBuilder::new(&allocator)
            .name(Ident::from("TestDirective"))
            .r#type(OutputAstBuilder::variable(&allocator, Ident::from("TestDirective")))
            .extract_from_class(&allocator, class.unwrap());

        let metadata = builder.build();
        assert!(metadata.is_some());

        let metadata = metadata.unwrap();
        assert_eq!(metadata.inputs.len(), 3);

        // Check first input
        assert_eq!(metadata.inputs[0].class_property_name.as_str(), "name");
        assert_eq!(metadata.inputs[0].binding_property_name.as_str(), "name");
        assert!(!metadata.inputs[0].required);

        // Check aliased input
        assert_eq!(metadata.inputs[1].class_property_name.as_str(), "value");
        assert_eq!(metadata.inputs[1].binding_property_name.as_str(), "aliasedValue");

        // Check required input
        assert_eq!(metadata.inputs[2].class_property_name.as_str(), "requiredInput");
        assert!(metadata.inputs[2].required);
    }

    #[test]
    fn test_extract_from_class_with_outputs() {
        let allocator = Allocator::default();
        let code = r#"
            class TestDirective {
                @Output() clicked = new EventEmitter<void>();
                @Output('valueChanged') onChange = new EventEmitter<string>();
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let builder = R3DirectiveMetadataBuilder::new(&allocator)
            .name(Ident::from("TestDirective"))
            .r#type(OutputAstBuilder::variable(&allocator, Ident::from("TestDirective")))
            .extract_from_class(&allocator, class.unwrap());

        let metadata = builder.build();
        assert!(metadata.is_some());

        let metadata = metadata.unwrap();
        assert_eq!(metadata.outputs.len(), 2);

        // Check first output
        assert_eq!(metadata.outputs[0].0.as_str(), "clicked");
        assert_eq!(metadata.outputs[0].1.as_str(), "clicked");

        // Check aliased output
        assert_eq!(metadata.outputs[1].0.as_str(), "onChange");
        assert_eq!(metadata.outputs[1].1.as_str(), "valueChanged");
    }

    #[test]
    fn test_extract_from_class_with_view_queries() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @ViewChild(ChildComponent) child: ChildComponent;
                @ViewChildren(ItemComponent) items: QueryList<ItemComponent>;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let builder = R3DirectiveMetadataBuilder::new(&allocator)
            .name(Ident::from("TestComponent"))
            .r#type(OutputAstBuilder::variable(&allocator, Ident::from("TestComponent")))
            .extract_from_class(&allocator, class.unwrap());

        let metadata = builder.build();
        assert!(metadata.is_some());

        let metadata = metadata.unwrap();
        assert_eq!(metadata.view_queries.len(), 2);

        // ViewChild has first = true
        assert_eq!(metadata.view_queries[0].property_name.as_str(), "child");
        assert!(metadata.view_queries[0].first);

        // ViewChildren has first = false
        assert_eq!(metadata.view_queries[1].property_name.as_str(), "items");
        assert!(!metadata.view_queries[1].first);
    }

    #[test]
    fn test_extract_from_class_with_content_queries() {
        let allocator = Allocator::default();
        let code = r#"
            class TestComponent {
                @ContentChild(PanelComponent) panel: PanelComponent;
                @ContentChildren(TabComponent) tabs: QueryList<TabComponent>;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let builder = R3DirectiveMetadataBuilder::new(&allocator)
            .name(Ident::from("TestComponent"))
            .r#type(OutputAstBuilder::variable(&allocator, Ident::from("TestComponent")))
            .extract_from_class(&allocator, class.unwrap());

        let metadata = builder.build();
        assert!(metadata.is_some());

        let metadata = metadata.unwrap();
        assert_eq!(metadata.queries.len(), 2);

        // ContentChild has first = true
        assert_eq!(metadata.queries[0].property_name.as_str(), "panel");
        assert!(metadata.queries[0].first);

        // ContentChildren has first = false
        assert_eq!(metadata.queries[1].property_name.as_str(), "tabs");
        assert!(!metadata.queries[1].first);
    }

    #[test]
    fn test_extract_from_class_with_host_bindings() {
        let allocator = Allocator::default();
        let code = r#"
            class TestDirective {
                @HostBinding('class.active') isActive: boolean;
                @HostBinding('attr.role') role: string = 'button';
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let builder = R3DirectiveMetadataBuilder::new(&allocator)
            .name(Ident::from("TestDirective"))
            .r#type(OutputAstBuilder::variable(&allocator, Ident::from("TestDirective")))
            .extract_from_class(&allocator, class.unwrap());

        let metadata = builder.build();
        assert!(metadata.is_some());

        let metadata = metadata.unwrap();
        assert_eq!(metadata.host.properties.len(), 2);

        // Check host bindings - keys are wrapped with brackets
        assert_eq!(metadata.host.properties[0].0.as_str(), "[class.active]");
        assert_eq!(metadata.host.properties[0].1.as_str(), "isActive");

        assert_eq!(metadata.host.properties[1].0.as_str(), "[attr.role]");
        assert_eq!(metadata.host.properties[1].1.as_str(), "role");
    }

    #[test]
    fn test_extract_from_class_with_host_listeners() {
        let allocator = Allocator::default();
        let code = r#"
            class TestDirective {
                @HostListener('click')
                onClick() {}

                @HostListener('mouseenter')
                onMouseEnter() {}
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let builder = R3DirectiveMetadataBuilder::new(&allocator)
            .name(Ident::from("TestDirective"))
            .r#type(OutputAstBuilder::variable(&allocator, Ident::from("TestDirective")))
            .extract_from_class(&allocator, class.unwrap());

        let metadata = builder.build();
        assert!(metadata.is_some());

        let metadata = metadata.unwrap();
        assert_eq!(metadata.host.listeners.len(), 2);

        // Check host listeners - keys are wrapped with parentheses, method expressions include ()
        assert_eq!(metadata.host.listeners[0].0.as_str(), "(click)");
        assert_eq!(metadata.host.listeners[0].1.as_str(), "onClick()");

        assert_eq!(metadata.host.listeners[1].0.as_str(), "(mouseenter)");
        assert_eq!(metadata.host.listeners[1].1.as_str(), "onMouseEnter()");
    }

    #[test]
    fn test_extract_from_class_with_all_decorator_types() {
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

        let builder = R3DirectiveMetadataBuilder::new(&allocator)
            .name(Ident::from("TestComponent"))
            .r#type(OutputAstBuilder::variable(&allocator, Ident::from("TestComponent")))
            .extract_from_class(&allocator, class.unwrap());

        let metadata = builder.build();
        assert!(metadata.is_some());

        let metadata = metadata.unwrap();

        // Verify all decorator types were extracted
        assert_eq!(metadata.inputs.len(), 1);
        assert_eq!(metadata.outputs.len(), 1);
        assert_eq!(metadata.view_queries.len(), 1);
        assert_eq!(metadata.queries.len(), 1);
        assert_eq!(metadata.host.properties.len(), 1);
        assert_eq!(metadata.host.listeners.len(), 1);
    }

    #[test]
    fn test_extract_from_class_empty_class() {
        let allocator = Allocator::default();
        let code = r#"
            class EmptyDirective {
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        let builder = R3DirectiveMetadataBuilder::new(&allocator)
            .name(Ident::from("EmptyDirective"))
            .r#type(OutputAstBuilder::variable(&allocator, Ident::from("EmptyDirective")))
            .extract_from_class(&allocator, class.unwrap());

        let metadata = builder.build();
        assert!(metadata.is_some());

        let metadata = metadata.unwrap();

        // All collections should be empty
        assert_eq!(metadata.inputs.len(), 0);
        assert_eq!(metadata.outputs.len(), 0);
        assert_eq!(metadata.view_queries.len(), 0);
        assert_eq!(metadata.queries.len(), 0);
        assert_eq!(metadata.host.properties.len(), 0);
        assert_eq!(metadata.host.listeners.len(), 0);
    }

    #[test]
    fn test_extract_from_class_preserves_existing_metadata() {
        let allocator = Allocator::default();
        let code = r#"
            class TestDirective {
                @Input() fromClass: string;
            }
        "#;

        let class = parse_class(&allocator, code);
        assert!(class.is_some());

        // Pre-add an input before extract_from_class
        let builder = R3DirectiveMetadataBuilder::new(&allocator)
            .name(Ident::from("TestDirective"))
            .r#type(OutputAstBuilder::variable(&allocator, Ident::from("TestDirective")))
            .add_input(R3InputMetadata::simple(Ident::from("existingInput")))
            .extract_from_class(&allocator, class.unwrap());

        let metadata = builder.build();
        assert!(metadata.is_some());

        let metadata = metadata.unwrap();

        // Should have both the pre-added input and the extracted one
        assert_eq!(metadata.inputs.len(), 2);
        assert_eq!(metadata.inputs[0].class_property_name.as_str(), "existingInput");
        assert_eq!(metadata.inputs[1].class_property_name.as_str(), "fromClass");
    }
}
