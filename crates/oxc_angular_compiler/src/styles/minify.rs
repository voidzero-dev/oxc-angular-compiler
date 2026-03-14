use lightningcss::stylesheet::{MinifyOptions, ParserOptions, PrinterOptions, StyleSheet};

const COMPONENT_PLACEHOLDER: &str = "%COMP%";
const MINIFY_PLACEHOLDER: &str = "OXCANGULARCOMPONENT";

/// Apply Angular style encapsulation and optionally minify the final CSS.
pub fn finalize_component_style(
    style: &str,
    encapsulate: bool,
    content_attr: &str,
    host_attr: &str,
    minify: bool,
) -> String {
    let style = if encapsulate {
        super::shim_css_text(style, content_attr, host_attr)
    } else {
        style.to_string()
    };

    if !minify || style.trim().is_empty() {
        return style;
    }

    minify_component_style(&style).unwrap_or(style)
}

/// Minify a final component CSS string while preserving Angular's `%COMP%` placeholder.
pub fn minify_component_style(style: &str) -> Option<String> {
    let css = style.replace(COMPONENT_PLACEHOLDER, MINIFY_PLACEHOLDER);
    let mut stylesheet = StyleSheet::parse(&css, ParserOptions::default()).ok()?;
    stylesheet.minify(MinifyOptions::default()).ok()?;

    let code =
        stylesheet.to_css(PrinterOptions { minify: true, ..PrinterOptions::default() }).ok()?.code;

    Some(code.replace(MINIFY_PLACEHOLDER, COMPONENT_PLACEHOLDER))
}

#[cfg(test)]
mod tests {
    use super::{finalize_component_style, minify_component_style};

    #[test]
    fn minifies_css_with_component_placeholders() {
        let minified = minify_component_style(
            "[_ngcontent-%COMP%] {\n  color: red;\n  background: transparent;\n}\n",
        )
        .expect("style should minify");

        assert_eq!(minified, "[_ngcontent-%COMP%]{color:red;background:0 0}");
    }

    #[test]
    fn finalizes_emulated_styles_before_minifying() {
        let finalized = finalize_component_style(
            ":host {\n  display: block;\n}\n.button {\n  color: red;\n}\n",
            true,
            "_ngcontent-%COMP%",
            "_nghost-%COMP%",
            true,
        );

        assert_eq!(
            finalized,
            "[_nghost-%COMP%]{display:block}.button[_ngcontent-%COMP%]{color:red}"
        );
    }
}
