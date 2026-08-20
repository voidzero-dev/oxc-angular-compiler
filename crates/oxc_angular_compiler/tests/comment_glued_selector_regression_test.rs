//! Regression test for a real-world encapsulation bug: a component's CSS
//! rule shipped with no `[_ngcontent-*]` scoping attribute at all, so it
//! matched (and visually clobbered) any element with the same class name
//! anywhere else in the app, with no console error.
//!
//! Root cause: when a Vite consumer preprocesses a component's `styleUrl`
//! file through PostCSS (e.g. for Tailwind) before handing it to this
//! crate's encapsulation pass, PostCSS's AST-based reprint collapses the
//! blank line that normally separates a `/* comment */` from the rule that
//! follows it. A hand-formatted stylesheet like:
//!
//! ```css
//! /* explains why this rule exists */
//! .widget {
//!     width: 9px;
//! }
//! ```
//!
//! reaches `shim_css_text` as `*/.widget { width: 9px; }` - the comment
//! glued directly to the selector with no separating whitespace.
//! `scope_simple_selector` used to bail out of scoping the *entire* selector
//! string whenever it contained the comment placeholder, so `.widget`
//! shipped unscoped: a plain, globally-matching rule any other component's
//! same-named class could collide with.
//!
//! Fixed in `scope_simple_selector` (`src/styles/encapsulation.rs`): leading
//! and trailing comment placeholders are now peeled off before scoping and
//! spliced back afterwards, instead of aborting scoping altogether.
//!
//! See `tests/shadow_css_test.rs` for the minimal unit-level regression
//! tests (`test_scope_selector_glued_directly_to_*`); this file exercises
//! the same bug against a full stylesheet shape with several unrelated,
//! comment-documented rules - the shape that originally surfaced it.

use oxc_angular_compiler::styles::finalize_component_style;

/// A demo stylesheet shaped like the one that surfaced this bug: several
/// unrelated rules, each documented with a `/* comment */` immediately
/// before it, and reproduced here exactly as PostCSS reprints it - with the
/// blank line that would normally separate the comment from the next rule
/// removed.
const CSS_WITH_COMMENTS_GLUED_TO_SELECTORS: &str = "\
:host { position: relative; overflow: hidden; }\
/* Contributes width but no height. */\
.probe { display: grid; height: 0; }\
:host:has(.widget) { padding-left: 21px; }\
.wrapper { overflow: auto; position: relative; }\
/* This rule positions the widget precisely. */\
.widget { width: 9px; padding: 0 4px; height: 100%; position: absolute; cursor: col-resize; }\
.widget:hover { background-color: red; }\
/* Sticky variant used when scrolling. */\
.container.sticky-header { display: flex; flex-direction: column; }\
";

#[test]
fn comment_glued_selectors_all_stay_scoped() {
    let result = finalize_component_style(
        CSS_WITH_COMMENTS_GLUED_TO_SELECTORS,
        true,
        "_ngcontent-%COMP%",
        "_nghost-%COMP%",
        false,
    );

    for selector in [".probe", ".wrapper", ".widget", ".container.sticky-header"] {
        let unscoped_rule_start = format!("{selector} {{");
        assert!(
            !result.contains(&unscoped_rule_start),
            "`{selector}` lost its Angular content-attribute scoping - a \
             comment glued directly to it (no separating whitespace, as \
             PostCSS reprints it) made `scope_simple_selector` bail out of \
             scoping the whole rule. Full shimmed output:\n{result}"
        );

        let scoped_rule = format!("{selector}[_ngcontent-%COMP%]");
        assert!(
            result.contains(&scoped_rule),
            "expected `{scoped_rule}` in output, got:\n{result}"
        );
    }
}
