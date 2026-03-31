//! HMR initializer code generation.
//!
//! This module generates the initialization code that sets up HMR listeners
//! for each component.
//!
//! Ported from Angular's `packages/compiler/src/render3/r3_hmr_compiler.ts`.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_span::Ident;

use super::dependencies::HmrMetadata;
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, BinaryOperator, BinaryOperatorExpr, DeclareFunctionStmt,
    DeclareVarStmt, DynamicImportExpr, DynamicImportUrl, ExpressionStatement, FnParam,
    InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr, LiteralValue, OutputExpression,
    OutputStatement, ReadKeyExpr, ReadPropExpr, ReadVarExpr, StmtModifier, TypeofExpr,
};
use crate::r3::Identifiers;

// ============================================================================
// HMR Initializer Compilation
// ============================================================================

/// Compiles the expression that initializes HMR for a class.
///
/// This generates an IIFE (immediately invoked function expression) that:
/// 1. Declares a unique ID for the component
/// 2. Creates a load function (`Cmp_HmrLoad`) that dynamically imports the update module
/// 3. Sets up a hot module listener for component updates
///
/// See: `packages/compiler/src/render3/r3_hmr_compiler.ts:55-157`
pub fn compile_hmr_initializer<'a>(
    allocator: &'a Allocator,
    meta: &HmrMetadata<'a>,
) -> OutputExpression<'a> {
    let module_name = "m";
    let data_name = "d";
    let timestamp_name = "t";
    let id_name = "id";
    let import_callback_name = format!("{}_HmrLoad", meta.class_name);

    // Build namespace array from dependencies
    // Each namespace dependency has an assigned_name (like "i0") that we use as a variable reference
    let namespaces: Vec<'a, OutputExpression<'a>> = Vec::from_iter_in(
        meta.namespace_dependencies.iter().map(|dep| {
            // Use the assigned_name (e.g., "i0") as a variable reference
            read_var(allocator, &dep.assigned_name)
        }),
        allocator,
    );

    // Build local dependencies array
    let locals: Vec<'a, OutputExpression<'a>> = Vec::from_iter_in(
        meta.local_dependencies.iter().map(|l| l.runtime_representation.clone_in(allocator)),
        allocator,
    );

    // m.default
    let default_read = read_prop(allocator, read_var(allocator, module_name), "default");

    // i0.ɵɵreplaceMetadata(Comp, m.default, [...namespaces], [...locals], import.meta, id)
    let replace_call = invoke_fn(
        allocator,
        read_prop(allocator, read_var(allocator, "i0"), Identifiers::REPLACE_METADATA),
        vec![
            meta.component_type.clone_in(allocator),
            default_read.clone_in(allocator),
            literal_arr(allocator, namespaces),
            literal_arr(allocator, locals),
            read_prop(allocator, read_var(allocator, "import"), "meta"),
            read_var(allocator, id_name),
        ],
    );

    // (m) => m.default && ɵɵreplaceMetadata(...)
    let replace_callback = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: Vec::from_iter_in([FnParam { name: Ident::from(module_name) }], allocator),
            body: ArrowFunctionBody::Expression(Box::new_in(
                binary_op(allocator, BinaryOperator::And, default_read, replace_call),
                allocator,
            )),
            source_span: None,
        },
        allocator,
    ));

    // i0.ɵɵgetReplaceMetadataURL(id, timestamp, import.meta.url)
    let url = invoke_fn(
        allocator,
        read_prop(allocator, read_var(allocator, "i0"), Identifiers::GET_REPLACE_METADATA_URL),
        vec![
            read_var(allocator, id_name),
            read_var(allocator, timestamp_name),
            read_prop(
                allocator,
                read_prop(allocator, read_var(allocator, "import"), "meta"),
                "url",
            ),
        ],
    );

    // import(/* @vite-ignore */ url).then((m) => ...)
    let dynamic_import = OutputExpression::DynamicImport(Box::new_in(
        DynamicImportExpr {
            url: DynamicImportUrl::Expression(Box::new_in(url, allocator)),
            url_comment: Some(Ident::from("@vite-ignore")),
            source_span: None,
        },
        allocator,
    ));

    let import_then_call =
        invoke_fn(allocator, read_prop(allocator, dynamic_import, "then"), vec![replace_callback]);

    // function Cmp_HmrLoad(t) { import(...).then(...); }
    let import_callback = OutputStatement::DeclareFunction(Box::new_in(
        DeclareFunctionStmt {
            name: Ident::from(allocator.alloc_str(&import_callback_name)),
            params: Vec::from_iter_in([FnParam { name: Ident::from(timestamp_name) }], allocator),
            statements: Vec::from_iter_in([expr_stmt(allocator, import_then_call)], allocator),
            modifiers: StmtModifier::FINAL,
            source_span: None,
        },
        allocator,
    ));

    // (d) => d.id === id && Cmp_HmrLoad(d.timestamp)
    let update_callback = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: Vec::from_iter_in([FnParam { name: Ident::from(data_name) }], allocator),
            body: ArrowFunctionBody::Expression(Box::new_in(
                binary_op(
                    allocator,
                    BinaryOperator::And,
                    binary_op(
                        allocator,
                        BinaryOperator::Identical,
                        read_prop(allocator, read_var(allocator, data_name), "id"),
                        read_var(allocator, id_name),
                    ),
                    invoke_fn(
                        allocator,
                        read_var(allocator, &import_callback_name),
                        vec![read_prop(allocator, read_var(allocator, data_name), "timestamp")],
                    ),
                ),
                allocator,
            )),
            source_span: None,
        },
        allocator,
    ));

    // Cmp_HmrLoad(Date.now())
    let initial_call = invoke_fn(
        allocator,
        read_var(allocator, &import_callback_name),
        vec![invoke_fn(
            allocator,
            read_prop(allocator, read_var(allocator, "Date"), "now"),
            vec![],
        )],
    );

    // import.meta.hot
    let hot_read =
        read_prop(allocator, read_prop(allocator, read_var(allocator, "import"), "meta"), "hot");

    // import.meta.hot.on('angular:component-update', updateCallback)
    let hot_listener = invoke_fn(
        allocator,
        read_prop(allocator, hot_read.clone_in(allocator), "on"),
        vec![literal_str(allocator, "angular:component-update"), update_callback],
    );

    // (d) => d.id === id && location.reload()
    // Handles the angular:invalidate event sent when HMR fails
    let invalidate_callback = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: Vec::from_iter_in([FnParam { name: Ident::from(data_name) }], allocator),
            body: ArrowFunctionBody::Expression(Box::new_in(
                binary_op(
                    allocator,
                    BinaryOperator::And,
                    binary_op(
                        allocator,
                        BinaryOperator::Identical,
                        read_prop(allocator, read_var(allocator, data_name), "id"),
                        read_var(allocator, id_name),
                    ),
                    invoke_fn(
                        allocator,
                        read_prop(allocator, read_var(allocator, "location"), "reload"),
                        vec![],
                    ),
                ),
                allocator,
            )),
            source_span: None,
        },
        allocator,
    ));

    // import.meta.hot.on('angular:invalidate', invalidateCallback)
    let invalidate_listener = invoke_fn(
        allocator,
        read_prop(allocator, hot_read.clone_in(allocator), "on"),
        vec![literal_str(allocator, "angular:invalidate"), invalidate_callback],
    );

    // Build the component ID - matches TypeScript's:
    // o.literal(encodeURIComponent(`${meta.filePath}@${meta.className}`))
    let component_id = encode_uri_component(&format!("{}@{}", meta.file_path, meta.class_name));

    // const id = '<encoded-id>';
    let id_decl = var_decl(allocator, id_name, literal_str(allocator, &component_id), true);

    // ngDevMode && Cmp_HmrLoad(Date.now());
    let guarded_initial_call = dev_only_guarded(allocator, initial_call);

    // import.meta.hot.accept(() => {})
    // Creates an HMR boundary in Vite so that:
    // 1. import.meta.hot is available for custom event listeners
    // 2. Module changes don't propagate up the importer chain
    // The empty callback means we handle updates via custom events, not accept() itself
    let empty_callback = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: Vec::new_in(allocator),
            body: ArrowFunctionBody::Statements(Vec::new_in(allocator)),
            source_span: None,
        },
        allocator,
    ));
    let hot_accept = invoke_fn(
        allocator,
        read_prop(allocator, hot_read.clone_in(allocator), "accept"),
        vec![empty_callback],
    );

    // ngDevMode && import.meta.hot && import.meta.hot.accept(() => {})
    let guarded_accept = dev_only_guarded(
        allocator,
        binary_op(allocator, BinaryOperator::And, hot_read.clone_in(allocator), hot_accept),
    );

    // ngDevMode && import.meta.hot && import.meta.hot.on('angular:component-update', ...)
    let guarded_listener = dev_only_guarded(
        allocator,
        binary_op(allocator, BinaryOperator::And, hot_read.clone_in(allocator), hot_listener),
    );

    // ngDevMode && import.meta.hot && import.meta.hot.on('angular:invalidate', ...)
    let guarded_invalidate_listener = dev_only_guarded(
        allocator,
        binary_op(allocator, BinaryOperator::And, hot_read, invalidate_listener),
    );

    // Build the IIFE: (() => { ... })()
    let body: Vec<'a, OutputStatement<'a>> = Vec::from_iter_in(
        [
            id_decl,
            import_callback,
            expr_stmt(allocator, guarded_initial_call),
            expr_stmt(allocator, guarded_accept),
            expr_stmt(allocator, guarded_listener),
            expr_stmt(allocator, guarded_invalidate_listener),
        ],
        allocator,
    );

    let iife = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: Vec::new_in(allocator),
            body: ArrowFunctionBody::Statements(body),
            source_span: None,
        },
        allocator,
    ));

    // Call the IIFE
    invoke_fn(allocator, iife, vec![])
}

// ============================================================================
// HMR Update Callback Compilation
// ============================================================================

/// Definition info for HMR update callback.
#[derive(Debug)]
pub struct HmrDefinition<'a> {
    /// Name of the field (e.g., "ɵcmp", "ɵfac").
    pub name: Ident<'a>,
    /// Initializer expression.
    pub initializer: Option<OutputExpression<'a>>,
    /// Additional statements after the field assignment.
    pub statements: Vec<'a, OutputStatement<'a>>,
}

/// Compiles the HMR update callback for a class.
///
/// The update callback receives:
/// 1. The component class
/// 2. An array of namespace modules (`ɵɵnamespaces`)
/// 3. Any local dependencies
///
/// It then extracts the individual namespaces and re-assigns the component definitions.
///
/// See: `packages/compiler/src/render3/r3_hmr_compiler.ts:166-210`
pub fn compile_hmr_update_callback<'a>(
    allocator: &'a Allocator,
    definitions: Vec<'a, HmrDefinition<'a>>,
    constant_statements: Vec<'a, OutputStatement<'a>>,
    meta: &HmrMetadata<'a>,
) -> OutputStatement<'a> {
    let namespaces_param = "ɵɵnamespaces";

    // Build function parameters: [className, ɵɵnamespaces, ...locals]
    let mut params: Vec<'a, FnParam<'a>> = Vec::new_in(allocator);
    params.push(FnParam { name: meta.class_name.clone() });
    params.push(FnParam { name: Ident::from(namespaces_param) });
    for local in &meta.local_dependencies {
        params.push(FnParam { name: local.name.clone() });
    }

    let mut body: Vec<'a, OutputStatement<'a>> = Vec::new_in(allocator);

    // Declare variables that read out the individual namespaces
    // const i0 = ɵɵnamespaces[0];
    // const i1 = ɵɵnamespaces[1];
    for (i, dep) in meta.namespace_dependencies.iter().enumerate() {
        let namespace_read = OutputExpression::ReadKey(Box::new_in(
            ReadKeyExpr {
                receiver: Box::new_in(read_var(allocator, namespaces_param), allocator),
                index: Box::new_in(literal_num(allocator, i as f64), allocator),
                optional: false,
                source_span: None,
            },
            allocator,
        ));
        body.push(var_decl(allocator, dep.assigned_name.as_str(), namespace_read, true));
    }

    // Add constant statements (takes ownership)
    for stmt in constant_statements {
        body.push(stmt);
    }

    // Add field assignments: ClassName.ɵcmp = defineComponent(...);
    for def in definitions {
        if let Some(initializer) = def.initializer {
            let assignment = binary_op(
                allocator,
                BinaryOperator::Assign,
                read_prop(
                    allocator,
                    read_var(allocator, meta.class_name.as_str()),
                    def.name.as_str(),
                ),
                initializer,
            );
            body.push(expr_stmt(allocator, assignment));

            // Add any additional statements (takes ownership)
            for stmt in def.statements {
                body.push(stmt);
            }
        }
    }

    // function ClassName_UpdateMetadata(ClassName, ɵɵnamespaces, ...locals) { ... }
    let fn_name = format!("{}_UpdateMetadata", meta.class_name);
    OutputStatement::DeclareFunction(Box::new_in(
        DeclareFunctionStmt {
            name: Ident::from(allocator.alloc_str(&fn_name)),
            params,
            statements: body,
            modifiers: StmtModifier::FINAL,
            source_span: None,
        },
        allocator,
    ))
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Create a read variable expression.
fn read_var<'a>(allocator: &'a Allocator, name: &str) -> OutputExpression<'a> {
    OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from(allocator.alloc_str(name)), source_span: None },
        allocator,
    ))
}

/// Create a property read expression.
fn read_prop<'a>(
    allocator: &'a Allocator,
    receiver: OutputExpression<'a>,
    name: &str,
) -> OutputExpression<'a> {
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(receiver, allocator),
            name: Ident::from(allocator.alloc_str(name)),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Create a function invocation expression.
fn invoke_fn<'a>(
    allocator: &'a Allocator,
    fn_expr: OutputExpression<'a>,
    args: std::vec::Vec<OutputExpression<'a>>,
) -> OutputExpression<'a> {
    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, allocator),
            args: Vec::from_iter_in(args, allocator),
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Create a string literal expression.
fn literal_str<'a>(allocator: &'a Allocator, value: &str) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr {
            value: LiteralValue::String(Ident::from(allocator.alloc_str(value))),
            source_span: None,
        },
        allocator,
    ))
}

/// Create a number literal expression.
fn literal_num<'a>(allocator: &'a Allocator, value: f64) -> OutputExpression<'a> {
    OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(value), source_span: None },
        allocator,
    ))
}

/// Create a literal array expression.
fn literal_arr<'a>(
    allocator: &'a Allocator,
    entries: Vec<'a, OutputExpression<'a>>,
) -> OutputExpression<'a> {
    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries, source_span: None },
        allocator,
    ))
}

/// Create a binary operator expression.
fn binary_op<'a>(
    allocator: &'a Allocator,
    op: BinaryOperator,
    lhs: OutputExpression<'a>,
    rhs: OutputExpression<'a>,
) -> OutputExpression<'a> {
    OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: op,
            lhs: Box::new_in(lhs, allocator),
            rhs: Box::new_in(rhs, allocator),
            source_span: None,
        },
        allocator,
    ))
}

/// Create a variable declaration statement.
fn var_decl<'a>(
    allocator: &'a Allocator,
    name: &str,
    value: OutputExpression<'a>,
    is_final: bool,
) -> OutputStatement<'a> {
    let modifiers = if is_final { StmtModifier::FINAL } else { StmtModifier::NONE };
    OutputStatement::DeclareVar(Box::new_in(
        DeclareVarStmt {
            name: Ident::from(allocator.alloc_str(name)),
            value: Some(value),
            modifiers,
            source_span: None,
            leading_comment: None,
        },
        allocator,
    ))
}

/// Create an expression statement.
fn expr_stmt<'a>(allocator: &'a Allocator, expr: OutputExpression<'a>) -> OutputStatement<'a> {
    OutputStatement::Expression(Box::new_in(
        ExpressionStatement { expr, source_span: None },
        allocator,
    ))
}

/// Creates: (typeof ngDevMode === "undefined" || ngDevMode) && expr
///
/// This defensive guard pattern allows HMR to work when:
/// - `ngDevMode` is undefined (default in development)
/// - `ngDevMode` is explicitly true
///
/// The simple pattern `ngDevMode && expr` would short-circuit when ngDevMode
/// is undefined, preventing HMR listeners from being registered.
///
/// Ported from Angular's `devOnlyGuardedExpression` in `packages/compiler/src/util.ts`.
fn dev_only_guarded<'a>(
    allocator: &'a Allocator,
    expr: OutputExpression<'a>,
) -> OutputExpression<'a> {
    let guard_var = read_var(allocator, "ngDevMode");

    // typeof ngDevMode
    let typeof_guard = OutputExpression::Typeof(Box::new_in(
        TypeofExpr {
            expr: Box::new_in(guard_var.clone_in(allocator), allocator),
            source_span: None,
        },
        allocator,
    ));

    // typeof ngDevMode === "undefined"
    let guard_not_defined = binary_op(
        allocator,
        BinaryOperator::Identical,
        typeof_guard,
        literal_str(allocator, "undefined"),
    );

    // typeof ngDevMode === "undefined" || ngDevMode
    let guard_undefined_or_true =
        binary_op(allocator, BinaryOperator::Or, guard_not_defined, guard_var);

    // (typeof ngDevMode === "undefined" || ngDevMode) && expr
    binary_op(allocator, BinaryOperator::And, guard_undefined_or_true, expr)
}

/// URL encoding matching JavaScript's `encodeURIComponent()`.
///
/// Characters NOT encoded by encodeURIComponent:
/// A-Z a-z 0-9 - _ . ! ~ * ' ( )
/// All other characters are encoded as %XX (uppercase hex).
fn encode_uri_component(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            // Alphanumeric characters are not encoded
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => {
                result.push(byte as char);
            }
            // Safe characters not encoded by encodeURIComponent
            b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                result.push(byte as char);
            }
            // Everything else is percent-encoded
            _ => {
                result.push('%');
                result.push_str(&format!("{byte:02X}"));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_uri_component() {
        // Basic path and class name encoding
        assert_eq!(
            encode_uri_component("src/app/app.component.ts@AppComponent"),
            "src%2Fapp%2Fapp.component.ts%40AppComponent"
        );

        // Safe characters should not be encoded
        assert_eq!(encode_uri_component("abc-_.!~*'()123"), "abc-_.!~*'()123");

        // Space and other special characters should be encoded
        assert_eq!(encode_uri_component("hello world"), "hello%20world");
        assert_eq!(encode_uri_component("a=b&c=d"), "a%3Db%26c%3Dd");
        assert_eq!(encode_uri_component("test?query#hash"), "test%3Fquery%23hash");
    }
}
