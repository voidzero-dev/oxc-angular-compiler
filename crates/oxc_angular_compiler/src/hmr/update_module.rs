//! HMR update module generation.
//!
//! This module generates the update modules that are served by the Vite
//! dev server when a component's template or styles change.
//!
//! The generated modules export a function that Angular's `ɵɵreplaceMetadata`
//! can call to apply the updated component definition.

/// Options for generating HMR update modules.
#[derive(Debug, Clone)]
pub struct HmrUpdateModuleOptions<'a> {
    /// Component ID (path@ClassName).
    pub component_id: &'a str,

    /// Component class name.
    pub class_name: &'a str,

    /// Compiled template as JavaScript code.
    pub template_js: Option<&'a str>,

    /// Compiled styles (CSS strings).
    pub styles: Option<&'a [String]>,

    /// Constant declarations (child view functions, pooled constants) as JavaScript code.
    /// These are emitted before the component definition update.
    pub declarations_js: Option<&'a str>,

    /// The consts array as JavaScript code.
    /// This must be included to ensure the template function's constant references
    /// match the component definition's consts array.
    pub consts_js: Option<&'a str>,

    /// Whether to include full component metadata.
    pub include_full_metadata: bool,
}

/// Generate an HMR update module for a component.
///
/// This generates a JavaScript module that exports a function which updates
/// the component's definition. The function is called by Angular's
/// `ɵɵreplaceMetadata` runtime function.
///
/// # Generated Format
///
/// ```javascript
/// // HMR update for: path/to/component.ts@ComponentName
/// export default function ComponentName_UpdateMetadata(ComponentName, ɵɵnamespaces) {
///   const i0 = ɵɵnamespaces[0];
///   ComponentName.ɵcmp = i0.ɵɵdefineComponent({
///     ...ComponentName.ɵcmp,
///     template: function ComponentName_Template(rf, ctx) { ... },
///     styles: ["..."],
///   });
/// }
/// ```
///
/// # Arguments
///
/// * `options` - HMR update module options
///
/// # Returns
///
/// JavaScript code for the HMR update module.
pub fn generate_hmr_update_module(options: &HmrUpdateModuleOptions<'_>) -> String {
    generate_hmr_update_module_internal(
        options.component_id,
        options.class_name,
        options.template_js,
        options.styles,
        options.declarations_js,
        options.consts_js,
    )
}

/// Generate an HMR update module from a compiled template string.
///
/// This is a convenience function when you already have the template
/// compiled to JavaScript.
///
/// # Arguments
///
/// * `component_id` - Component ID (path@ClassName)
/// * `template_js` - Compiled template function as JavaScript
/// * `styles` - Optional array of CSS styles
/// * `declarations_js` - Optional constant declarations (child views, pooled constants)
///
/// # Returns
///
/// JavaScript code for the HMR update module.
pub fn generate_hmr_update_module_from_js(
    component_id: &str,
    template_js: &str,
    styles: Option<&[String]>,
    declarations_js: Option<&str>,
    consts_js: Option<&str>,
) -> String {
    // Extract class name from component_id (format: "path@ClassName")
    let class_name = component_id.split('@').nth(1).unwrap_or("Component");

    generate_hmr_update_module_internal(
        component_id,
        class_name,
        Some(template_js),
        styles,
        declarations_js,
        consts_js,
    )
}

/// Internal function to generate HMR update module.
fn generate_hmr_update_module_internal(
    component_id: &str,
    class_name: &str,
    template_js: Option<&str>,
    styles: Option<&[String]>,
    declarations_js: Option<&str>,
    consts_js: Option<&str>,
) -> String {
    let mut output = String::new();

    // Add comment with component ID
    output.push_str(&format!("// HMR update for: {}\n", component_id));

    // Export a function that Angular's ɵɵreplaceMetadata will call
    // The function signature matches what compileHmrUpdateCallback generates:
    // function ClassName_UpdateMetadata(ClassName, ɵɵnamespaces, ...locals)
    output.push_str(&format!(
        "export default function {}_UpdateMetadata({}, ɵɵnamespaces) {{\n",
        class_name, class_name
    ));

    // Destructure the Angular core namespace from namespaces array
    output.push_str("  const i0 = ɵɵnamespaces[0];\n");

    // Add constant declarations (child view functions, pooled constants)
    // These must be declared before the component definition since the template may reference them
    if let Some(declarations) = declarations_js {
        if !declarations.is_empty() {
            // Indent each line of declarations for consistent formatting
            for line in declarations.lines() {
                output.push_str("  ");
                output.push_str(line);
                output.push('\n');
            }
        }
    }

    // Update the component definition using ɵɵdefineComponent
    // We spread the existing definition and override template/styles.
    // IMPORTANT: We must override `inputs` with `inputConfig` because the spread
    // includes `inputs` in the already-processed format (output of
    // `parseAndConvertInputsForDefinition`). If we don't override, ɵɵdefineComponent
    // will process them again, producing corrupted input mappings.
    // `inputConfig` stores the original unprocessed inputs format.
    // Only override when inputConfig exists (components with inputs); otherwise
    // setting `inputs: undefined` would corrupt the component definition.
    output.push_str(&format!("  {}.ɵcmp = i0.ɵɵdefineComponent({{\n", class_name));
    output.push_str(&format!("    ...{}.ɵcmp,\n", class_name));
    output.push_str(&format!(
        "    ...({cn}.ɵcmp.inputConfig ? {{ inputs: {cn}.ɵcmp.inputConfig }} : {{}}),\n",
        cn = class_name
    ));

    // Add template function if present
    if let Some(template_js) = template_js {
        output.push_str("    template: ");
        output.push_str(template_js);
        output.push_str(",\n");
    }

    // Add consts array if present
    // This must be included to override the old consts from the spread, ensuring
    // the template function's constant references match the actual consts array
    if let Some(consts_js) = consts_js {
        output.push_str("    consts: ");
        output.push_str(consts_js);
        output.push_str(",\n");
    }

    // Add styles if present
    if let Some(styles) = styles {
        if !styles.is_empty() {
            output.push_str("    styles: [\n");
            for style in styles {
                output.push_str("      ");
                output.push_str(&format!("{:?}", style));
                output.push_str(",\n");
            }
            output.push_str("    ],\n");
        }
    }

    output.push_str("  });\n");
    output.push_str("}\n");

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_hmr_update_module_from_js() {
        let result = generate_hmr_update_module_from_js(
            "src/app/app.component.ts@AppComponent",
            "function AppComponent_Template(rf, ctx) { }",
            Some(&["h1 { color: red; }".to_string()]),
            None,
            None,
        );

        assert!(result.contains("export default function AppComponent_UpdateMetadata"));
        assert!(result.contains("const i0 = ɵɵnamespaces[0]"));
        assert!(result.contains("AppComponent.ɵcmp = i0.ɵɵdefineComponent"));
        assert!(result.contains("template:"));
        assert!(result.contains("styles:"));
        assert!(result.contains("color: red"));
    }

    #[test]
    fn test_generate_hmr_update_module_no_styles() {
        let result = generate_hmr_update_module_from_js(
            "src/app/app.component.ts@AppComponent",
            "function AppComponent_Template(rf, ctx) { }",
            None,
            None,
            None,
        );

        assert!(result.contains("export default function AppComponent_UpdateMetadata"));
        assert!(result.contains("template:"));
        assert!(!result.contains("styles:"));
    }

    #[test]
    fn test_class_name_extraction() {
        let result = generate_hmr_update_module_from_js(
            "path/to/my.component.ts@MyComponent",
            "function MyComponent_Template(rf, ctx) { }",
            None,
            None,
            None,
        );

        assert!(result.contains("function MyComponent_UpdateMetadata(MyComponent"));
    }

    #[test]
    fn test_generate_hmr_update_module_with_declarations() {
        let declarations = "function App_For_1(rf, ctx) { }\nconst _c0 = [1, 2, 3];";
        let result = generate_hmr_update_module_from_js(
            "src/app/app.component.ts@AppComponent",
            "function AppComponent_Template(rf, ctx) { App_For_1(rf, ctx); }",
            None,
            Some(declarations),
            None,
        );

        assert!(result.contains("export default function AppComponent_UpdateMetadata"));
        assert!(result.contains("function App_For_1(rf, ctx)"));
        assert!(result.contains("const _c0 = [1, 2, 3]"));
        // Declarations should come before component definition
        let decl_pos = result.find("App_For_1").unwrap();
        let cmp_pos = result.find("ɵcmp").unwrap();
        assert!(decl_pos < cmp_pos);
    }

    #[test]
    fn test_generate_hmr_update_module_uses_input_config() {
        let result = generate_hmr_update_module_from_js(
            "src/app/app.component.ts@AppComponent",
            "function AppComponent_Template(rf, ctx) { }",
            None,
            None,
            None,
        );

        // The HMR module must conditionally override `inputs` with `inputConfig`
        // to avoid double-processing by `parseAndConvertInputsForDefinition`.
        // It must only do so when inputConfig exists to avoid setting inputs to undefined.
        assert!(result.contains("AppComponent.ɵcmp.inputConfig"));
        assert!(result.contains("inputs:"));

        // `inputs` override must come AFTER the spread to take precedence
        let spread_pos = result.find("...AppComponent.ɵcmp").unwrap();
        let inputs_pos = result.find("inputConfig").unwrap();
        assert!(inputs_pos > spread_pos);
    }

    #[test]
    fn test_generate_hmr_update_module_with_consts() {
        let result = generate_hmr_update_module_from_js(
            "src/app/app.component.ts@AppComponent",
            "function AppComponent_Template(rf, ctx) { }",
            None,
            None,
            Some("[\"value1\", 42, [1, 2, 3]]"),
        );

        assert!(result.contains("export default function AppComponent_UpdateMetadata"));
        assert!(result.contains("consts: [\"value1\", 42, [1, 2, 3]]"));
    }
}
