//! HMR update module generation.
//!
//! This module generates the update modules that are served by the Vite
//! dev server when a component's template or styles change.
//!
//! The generated modules export a function that Angular's `ɵɵreplaceMetadata`
//! can call to apply the updated component definition.

/// The fields of a component definition that its template decides, from a compile of
/// the new template.
#[derive(Debug, Clone, Copy)]
pub struct HmrTemplateFields<'a> {
    /// The compiled template function as JavaScript code.
    pub template_js: &'a str,

    /// The number of element, text and container slots the template creates.
    pub decls: u32,

    /// The number of binding slots the template uses.
    pub vars: u32,

    /// The consts array (or the function returning it) as JavaScript code, if the
    /// template has any.
    pub consts_js: Option<&'a str>,

    /// The `ngContentSelectors` array as JavaScript code, if the template projects
    /// content.
    pub ng_content_selectors_js: Option<&'a str>,

    /// Whether the new template needs `ɵɵAnimationsFeature()` (an `animate.leave`
    /// on v20.2.0 to v20.3.0). ngtsc's update module runs `ɵɵdefineComponent`
    /// again, which runs the feature and sets up the registry `ɵɵanimateLeave`
    /// needs. The live definition is copied here instead, so the module runs
    /// the feature itself.
    pub animations_feature: bool,
}

/// Options for generating HMR update modules.
#[derive(Debug, Clone)]
pub struct HmrUpdateModuleOptions<'a> {
    /// Component ID (path@ClassName).
    pub component_id: &'a str,

    /// Component class name.
    pub class_name: &'a str,

    /// The new template's fields, or `None` when only the styles changed.
    pub template: Option<HmrTemplateFields<'a>>,

    /// Compiled styles (CSS strings).
    pub styles: Option<&'a [String]>,

    /// Constant declarations (child view functions, pooled constants) as JavaScript code.
    /// These are emitted before the component definition update.
    pub declarations_js: Option<&'a str>,
}

/// Generate an HMR update module for a component.
///
/// This generates a JavaScript module that exports a function which updates
/// the component's definition. The function is called by Angular's
/// `ɵɵreplaceMetadata` runtime function, which merges the definition it leaves
/// on the class into the live one and re-creates the component's views.
///
/// # Generated Format
///
/// ```javascript
/// // HMR update for: path/to/component.ts@ComponentName
/// export default function ComponentName_UpdateMetadata(ComponentName, ɵɵnamespaces) {
///   const i0 = ɵɵnamespaces[0];
///   ComponentName.ɵcmp = {
///     ...ComponentName.ɵcmp,
///     decls: 2,
///     vars: 1,
///     consts: null,
///     ngContentSelectors: undefined,
///     template: function ComponentName_Template(rf, ctx) { ... },
///     styles: ["..."],
///     tView: null,
///   };
/// }
/// ```
///
/// Angular's own update module (`compileHmrUpdateCallback`) writes a whole new
/// `ɵɵdefineComponent({...})` from the component class. Only the template is
/// compiled here, so the rest of the definition is the live one, copied as it
/// is. It must not go back through `ɵɵdefineComponent`: that function converts
/// `inputs` and `outputs` and derives `onPush` from `changeDetection`, so a
/// definition it has already built comes out with its outputs reversed and its
/// change detection strategy lost.
///
/// Every field the template decides is written, including the ones the new
/// template has no value for, because a key left out keeps the old template's.
/// `tView` is cleared so the runtime builds the view for the new template.
///
/// # Arguments
///
/// * `options` - HMR update module options
///
/// # Returns
///
/// JavaScript code for the HMR update module.
pub fn generate_hmr_update_module(options: &HmrUpdateModuleOptions<'_>) -> String {
    let mut fields = std::vec::Vec::new();
    if let Some(template) = &options.template {
        fields.push(("decls", template.decls.to_string()));
        fields.push(("vars", template.vars.to_string()));
        fields.push(("consts", template.consts_js.unwrap_or("null").to_string()));
        fields.push((
            "ngContentSelectors",
            template.ng_content_selectors_js.unwrap_or("undefined").to_string(),
        ));
        fields.push(("template", template.template_js.to_string()));
    }
    emit_update_module(
        options.component_id,
        options.class_name,
        &fields,
        options.styles,
        options.declarations_js,
        // A missing template is a styles-only update: the live tView still
        // matches the template it was built for, so keep it and let
        // recreateMatchingLViews re-render rather than rebuilding it.
        options.template.is_some(),
        options.template.is_some_and(|template| template.animations_feature),
    )
}

/// Generate an HMR update module from a compiled template string.
///
/// This is a convenience function when you already have the template
/// compiled to JavaScript. It knows only what it is given: the module replaces
/// the template function and `consts`, but `decls`, `vars` and
/// `ngContentSelectors` stay as the live definition has them — keys the module
/// does not write survive the `...ɵcmp` spread — so it is only correct for a
/// template that needs the same slot counts and projection selectors as the
/// old one. For a template that changes them, use
/// [`generate_hmr_update_module`] with the output of a template compile, which
/// writes them all.
///
/// # Arguments
///
/// * `component_id` - Component ID (path@ClassName)
/// * `template_js` - Compiled template function as JavaScript, or an empty
///   string for a styles-only update
/// * `styles` - Optional array of CSS styles
/// * `declarations_js` - Optional constant declarations (child views, pooled constants)
/// * `consts_js` - The new template's consts array as JavaScript. When the
///   module emits a template, `None` writes `consts: null`, clearing whatever
///   the old template's consts held.
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
    // Extract class name from component_id (format: "path@ClassName"). The path
    // can contain `@` (e.g. `node_modules/@scope/...`) but a class name cannot.
    let class_name = component_id.rsplit_once('@').map_or("Component", |(_, name)| name);

    let mut fields = std::vec::Vec::new();
    if !template_js.is_empty() {
        fields.push(("template", template_js.to_string()));
        // A swapped template decides the consts it indexes into, so an absent
        // `consts_js` clears them rather than keeping the old template's.
        fields.push(("consts", consts_js.unwrap_or("null").to_string()));
    }
    emit_update_module(
        component_id,
        class_name,
        &fields,
        styles,
        declarations_js,
        !template_js.is_empty(),
        false,
    )
}

/// Emits an update module that copies the live definition and overrides `fields`
/// (name and JavaScript value) and, when known, `styles`. `clear_tview` writes
/// `tView: null`, making the runtime build the view for the new template; it
/// must only be set when `fields` replaces the template. `animations_feature`
/// runs `ɵɵAnimationsFeature()` on the new definition (see
/// [`HmrTemplateFields::animations_feature`]).
fn emit_update_module(
    component_id: &str,
    class_name: &str,
    fields: &[(&str, String)],
    styles: Option<&[String]>,
    declarations_js: Option<&str>,
    clear_tview: bool,
    animations_feature: bool,
) -> String {
    let mut output = String::new();

    // Add comment with component ID
    output.push_str(&format!("// HMR update for: {component_id}\n"));

    // Export a function that Angular's ɵɵreplaceMetadata will call
    // The function signature matches what compileHmrUpdateCallback generates:
    // function ClassName_UpdateMetadata(ClassName, ɵɵnamespaces, ...locals)
    output.push_str(&format!(
        "export default function {class_name}_UpdateMetadata({class_name}, ɵɵnamespaces) {{\n"
    ));

    // Destructure the Angular core namespace from namespaces array
    output.push_str("  const i0 = ɵɵnamespaces[0];\n");

    // Add constant declarations (child view functions, pooled constants)
    // These must be declared before the component definition since the template may reference them
    if let Some(declarations) = declarations_js {
        // Indent each line of declarations for consistent formatting
        for line in declarations.lines() {
            output.push_str("  ");
            output.push_str(line);
            output.push('\n');
        }
    }

    output.push_str(&format!("  {class_name}.ɵcmp = {{\n"));
    output.push_str(&format!("    ...{class_name}.ɵcmp,\n"));

    for (name, value) in fields {
        output.push_str(&format!("    {name}: {value},\n"));
    }

    // Add styles whenever the caller has an answer — an EMPTY list included.
    // The definition above spreads `...ClassName.ɵcmp`, so every key this
    // module does not emit keeps its previous value. A component that lost its
    // last stylesheet therefore needs an explicit `styles: []` to clear it;
    // omitting the key would leave the old CSS applied until a full reload.
    // `None` stays omitted: that is "the caller does not know", not "empty".
    if let Some(styles) = styles {
        if styles.is_empty() {
            output.push_str("    styles: [],\n");
        } else {
            output.push_str("    styles: [\n");
            for style in styles {
                output.push_str("      ");
                output.push_str(&format!("{:?}", style));
                output.push_str(",\n");
            }
            output.push_str("    ],\n");
        }
    }

    if clear_tview {
        output.push_str("    tView: null,\n");
    }
    output.push_str("  };\n");
    if animations_feature {
        output.push_str(&format!("  i0.ɵɵAnimationsFeature()({class_name}.ɵcmp);\n"));
    }
    output.push_str("}\n");

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: HmrTemplateFields<'static> = HmrTemplateFields {
        template_js: "function AppComponent_Template(rf, ctx) { }",
        decls: 3,
        vars: 2,
        consts_js: None,
        ng_content_selectors_js: None,
        animations_feature: false,
    };

    fn module(
        component_id: &str,
        template: Option<HmrTemplateFields<'_>>,
        styles: Option<&[String]>,
        declarations_js: Option<&str>,
    ) -> String {
        // The path can contain `@` (e.g. `node_modules/@scope/...`); a class name cannot.
        let class_name = component_id.rsplit_once('@').map_or("Component", |(_, name)| name);
        generate_hmr_update_module(&HmrUpdateModuleOptions {
            component_id,
            class_name,
            template,
            styles,
            declarations_js,
        })
    }

    #[test]
    fn test_generate_hmr_update_module() {
        let result = module(
            "src/app/app.component.ts@AppComponent",
            Some(TEMPLATE),
            Some(&["h1 { color: red; }".to_string()]),
            None,
        );

        assert!(result.contains("export default function AppComponent_UpdateMetadata"));
        assert!(result.contains("const i0 = ɵɵnamespaces[0]"));
        assert!(result.contains("template: function AppComponent_Template(rf, ctx) { },"));
        assert!(result.contains("styles:"));
        assert!(result.contains("color: red"));
    }

    /// The live definition is copied, never handed back to `ɵɵdefineComponent`, which
    /// would convert its `inputs` and `outputs` a second time.
    #[test]
    fn test_live_definition_is_copied_not_redefined() {
        let result = module("src/app/app.component.ts@AppComponent", Some(TEMPLATE), None, None);

        assert!(result.contains("AppComponent.ɵcmp = {\n    ...AppComponent.ɵcmp,\n"));
        assert!(!result.contains("ɵɵdefineComponent"));
        assert!(!result.contains("inputConfig"));
        // The runtime builds the view for the new template only if none is cached.
        assert!(result.contains("    tView: null,\n  };"));
    }

    /// Every field the template decides is written after the spread, so none keeps the
    /// old template's value: not even one the new template has nothing for.
    #[test]
    fn test_template_fields_are_all_written() {
        let result = module("src/app/app.component.ts@AppComponent", Some(TEMPLATE), None, None);

        let spread = result.find("...AppComponent.ɵcmp").unwrap();
        for field in ["decls: 3,", "vars: 2,", "consts: null,", "ngContentSelectors: undefined,"] {
            let at = result.find(field).unwrap_or_else(|| panic!("missing `{field}`:\n{result}"));
            assert!(at > spread, "`{field}` must override the spread");
        }
    }

    #[test]
    fn test_consts_and_content_selectors() {
        let result = module(
            "src/app/app.component.ts@AppComponent",
            Some(HmrTemplateFields {
                consts_js: Some("[\"value1\", 42, [1, 2, 3]]"),
                ng_content_selectors_js: Some("_c1"),
                ..TEMPLATE
            }),
            None,
            Some("const _c0 = [[[\"\", \"x\", \"\"]]];\nconst _c1 = [\"[x]\"];"),
        );

        assert!(result.contains("consts: [\"value1\", 42, [1, 2, 3]],"));
        assert!(result.contains("ngContentSelectors: _c1,"));
    }

    /// A styles-only update leaves the template's fields, and the view built for it, alone.
    #[test]
    fn test_styles_only_update() {
        let result = module(
            "src/app/app.component.ts@AppComponent",
            None,
            Some(&["h1 { color: red; }".to_string()]),
            None,
        );

        // `tView` included: clearing it would make getOrCreateComponentTView
        // rebuild an identical view before recreateMatchingLViews re-renders.
        for field in ["decls:", "vars:", "consts:", "ngContentSelectors:", "template:", "tView:"] {
            assert!(!result.contains(field), "unexpected `{field}`:\n{result}");
        }
        assert!(result.contains("color: red"));
    }

    #[test]
    fn test_generate_hmr_update_module_no_styles() {
        let result = module("src/app/app.component.ts@AppComponent", Some(TEMPLATE), None, None);

        assert!(result.contains("export default function AppComponent_UpdateMetadata"));
        assert!(result.contains("template:"));
        assert!(!result.contains("styles:"));
    }

    #[test]
    fn test_generate_hmr_update_module_empty_styles_clears() {
        // An EMPTY list is a definitive answer, not silence. The module opens
        // with `...ClassName.ɵcmp`, so a key it does not emit keeps its old
        // value — omitting `styles` here would leave the component's last
        // stylesheet applied until a full reload.
        let result =
            module("src/app/app.component.ts@AppComponent", Some(TEMPLATE), Some(&[]), None);

        assert!(result.contains("styles: [],"));
    }

    #[test]
    fn test_class_name_in_function_and_definition() {
        for id in [
            "path/to/my.component.ts@MyComponent",
            "node_modules/@scope/pkg/src/a.ts@MyComponent",
            "packages/@a/@b/x.ts@MyComponent",
        ] {
            let result = module(id, Some(TEMPLATE), None, None);
            assert!(result.contains("function MyComponent_UpdateMetadata(MyComponent"), "{id}");
            assert!(result.contains("MyComponent.ɵcmp ="), "{id}");
            assert!(result.contains(&format!("// HMR update for: {id}\n")), "{id}");
        }
    }

    /// From template JavaScript alone the slot counts are unknown, so they are
    /// left as they are — `decls`, `vars` and `ngContentSelectors` keep the old
    /// template's values, which is only correct when they do not change.
    #[test]
    fn test_generate_hmr_update_module_from_js() {
        let result = generate_hmr_update_module_from_js(
            "node_modules/@scope/pkg/src/a.ts@MyComponent",
            "function MyComponent_Template(rf, ctx) { }",
            Some(&["h1 { color: red; }".to_string()]),
            None,
            Some("[\"value1\"]"),
        );

        assert!(result.contains("function MyComponent_UpdateMetadata(MyComponent"));
        assert!(result.contains("MyComponent.ɵcmp = {\n    ...MyComponent.ɵcmp,\n"));
        assert!(result.contains("template: function MyComponent_Template(rf, ctx) { },"));
        assert!(result.contains("consts: [\"value1\"],"));
        assert!(result.contains("color: red"));
        assert!(result.contains("    tView: null,\n  };"));
        assert!(!result.contains("ɵɵdefineComponent"));
        assert!(!result.contains("decls:"));
        assert!(!result.contains("ngContentSelectors:"));

        // The swapped template decides `consts`: no consts argument clears the
        // old template's rather than keeping it through the spread.
        let result = generate_hmr_update_module_from_js(
            "a.ts@MyComponent",
            "function T(rf, ctx) {}",
            None,
            None,
            None,
        );
        assert!(result.contains("consts: null,"), "{result}");

        // No template is a styles-only update: no template key (which would be
        // `template: ,` — invalid JS) and no `tView` clear.
        let result = generate_hmr_update_module_from_js(
            "MyComponent",
            "",
            Some(&["h1 { color: red; }".to_string()]),
            None,
            None,
        );
        assert!(result.contains("function Component_UpdateMetadata(Component"));
        assert!(!result.contains("template:"), "{result}");
        assert!(!result.contains("tView"), "{result}");
        assert!(result.contains("color: red"));
    }

    #[test]
    fn test_generate_hmr_update_module_with_declarations() {
        let declarations = "function App_For_1(rf, ctx) { }\nconst _c0 = [1, 2, 3];";
        let result = module(
            "src/app/app.component.ts@AppComponent",
            Some(HmrTemplateFields {
                template_js: "function AppComponent_Template(rf, ctx) { App_For_1(rf, ctx); }",
                ..TEMPLATE
            }),
            None,
            Some(declarations),
        );

        assert!(result.contains("export default function AppComponent_UpdateMetadata"));
        assert!(result.contains("function App_For_1(rf, ctx)"));
        assert!(result.contains("const _c0 = [1, 2, 3]"));
        // Declarations should come before component definition
        let decl_pos = result.find("App_For_1").unwrap();
        let cmp_pos = result.find("ɵcmp").unwrap();
        assert!(decl_pos < cmp_pos);
    }
}
