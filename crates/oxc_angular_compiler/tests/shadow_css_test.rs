//! ShadowCss conformance tests.
//!
//! These tests are ported from Angular's shadow_css test suite:
//! - packages/compiler/test/shadow_css/shadow_css_spec.ts
//! - packages/compiler/test/shadow_css/host_and_host_context_spec.ts
//! - packages/compiler/test/shadow_css/ng_deep_spec.ts
//! - packages/compiler/test/shadow_css/keyframes_spec.ts
//! - packages/compiler/test/shadow_css/at_rules_spec.ts
//!
//! The goal is to have 1:1 compatibility with Angular's ShadowCss implementation.

use oxc_angular_compiler::styles::shim_css_text;

/// Normalize CSS for comparison (matches Angular's toEqualCss behavior).
/// - Removes leading/trailing whitespace
/// - Collapses multiple whitespace to single space
/// - Removes space after colons
/// - Removes space before/after braces
fn normalize_css(css: &str) -> String {
    css.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(": ", ":")
        .replace(" }", "}")
        .replace(" {", "{")
}

/// Helper to shim CSS (matches Angular's shim() helper).
/// Angular uses:
/// - `contentAttr` for element scoping (e.g., "_ngcontent-xxx" or "contenta" in tests)
/// - `hostAttr` for host element scoping (e.g., "_nghost-xxx" or "a-host" in tests)
fn shim(css: &str, content_attr: &str) -> String {
    shim_css_text(css, content_attr, "")
}

/// Helper with both content and host attributes.
fn shim_with_host(css: &str, content_attr: &str, host_attr: &str) -> String {
    shim_css_text(css, content_attr, host_attr)
}

/// Assert CSS equality with normalization.
macro_rules! assert_css_eq {
    ($actual:expr, $expected:expr) => {
        let actual_normalized = normalize_css(&$actual);
        let expected_normalized = normalize_css($expected);
        assert_eq!(
            actual_normalized, expected_normalized,
            "\n\nActual (normalized):\n{}\n\nExpected (normalized):\n{}\n\nActual (raw):\n{}\n\nExpected (raw):\n{}",
            actual_normalized, expected_normalized, $actual, $expected
        );
    };
}

// ============================================================================
// Basic Element Tests (from shadow_css_spec.ts)
// ============================================================================

#[test]
fn test_handle_empty_string() {
    assert_css_eq!(shim("", "contenta"), "");
}

#[test]
fn test_add_attribute_to_every_rule() {
    let css = "one {color: red;}two {color: red;}";
    let expected = "one[contenta] {color:red;}two[contenta] {color:red;}";
    assert_css_eq!(shim(css, "contenta"), expected);
}

#[test]
fn test_handle_invalid_css() {
    let css = "one {color: red;}garbage";
    let expected = "one[contenta] {color:red;}garbage";
    assert_css_eq!(shim(css, "contenta"), expected);
}

#[test]
fn test_add_attribute_to_every_selector() {
    let css = "one, two {color: red;}";
    let expected = "one[contenta], two[contenta] {color:red;}";
    assert_css_eq!(shim(css, "contenta"), expected);
}

#[test]
fn test_support_newlines_in_selector_and_content() {
    let css = "
      one,
      two {
        color: red;
      }
    ";
    let expected = "
      one[contenta],
      two[contenta] {
        color: red;
      }
    ";
    assert_css_eq!(shim(css, "contenta"), expected);
}

#[test]
fn test_handle_complicated_selectors_pseudo_before() {
    assert_css_eq!(shim("one::before {}", "contenta"), "one[contenta]::before {}");
}

#[test]
fn test_handle_complicated_selectors_descendant() {
    assert_css_eq!(shim("one two {}", "contenta"), "one[contenta] two[contenta] {}");
}

#[test]
fn test_handle_complicated_selectors_child() {
    assert_css_eq!(shim("one > two {}", "contenta"), "one[contenta] > two[contenta] {}");
}

#[test]
fn test_handle_complicated_selectors_adjacent() {
    assert_css_eq!(shim("one + two {}", "contenta"), "one[contenta] + two[contenta] {}");
}

#[test]
fn test_handle_complicated_selectors_sibling() {
    assert_css_eq!(shim("one ~ two {}", "contenta"), "one[contenta] ~ two[contenta] {}");
}

#[test]
fn test_handle_compound_class_selectors() {
    assert_css_eq!(
        shim(".one.two > three {}", "contenta"),
        ".one.two[contenta] > three[contenta] {}"
    );
}

#[test]
fn test_handle_attribute_selectors() {
    assert_css_eq!(shim("one[attr=\"value\"] {}", "contenta"), "one[attr=\"value\"][contenta] {}");
}

#[test]
fn test_handle_attribute_selectors_no_quotes() {
    assert_css_eq!(shim("one[attr=value] {}", "contenta"), "one[attr=value][contenta] {}");
}

#[test]
fn test_handle_attribute_selectors_caret() {
    assert_css_eq!(
        shim("one[attr^=\"value\"] {}", "contenta"),
        "one[attr^=\"value\"][contenta] {}"
    );
}

#[test]
fn test_handle_attribute_selectors_dollar() {
    assert_css_eq!(
        shim("one[attr$=\"value\"] {}", "contenta"),
        "one[attr$=\"value\"][contenta] {}"
    );
}

#[test]
fn test_handle_attribute_selectors_star() {
    assert_css_eq!(
        shim("one[attr*=\"value\"] {}", "contenta"),
        "one[attr*=\"value\"][contenta] {}"
    );
}

#[test]
fn test_handle_standalone_attribute_selector() {
    assert_css_eq!(shim("[attr] {}", "contenta"), "[attr][contenta] {}");
}

// ============================================================================
// Pseudo Element Tests
// ============================================================================

#[test]
fn test_handle_pseudo_elements() {
    assert_css_eq!(
        shim(".button::before { content: ''; }", "contenta"),
        ".button[contenta]::before { content:''; }"
    );
}

// ============================================================================
// Media Query Tests (from at_rules_spec.ts)
// ============================================================================

#[test]
fn test_handle_media_query() {
    let css = "@media (max-width: 600px) { .button { color: red; } }";
    let expected = "@media (max-width: 600px) { .button[contenta] { color: red; } }";
    assert_css_eq!(shim(css, "contenta"), expected);
}

#[test]
fn test_preserve_keyframe_selectors() {
    // Keyframe selectors (from, to, percentages) should NOT be scoped
    let css = "@keyframes foo { from { opacity: 0; } to { opacity: 1; } }";
    // The keyframe selectors should remain unchanged
    let result = shim(css, "contenta");
    assert!(
        result.contains("from {") || result.contains("from{"),
        "Should preserve 'from' selector"
    );
    assert!(result.contains("to {") || result.contains("to{"), "Should preserve 'to' selector");
}

// ============================================================================
// :host Tests (from host_and_host_context_spec.ts)
// ============================================================================

#[test]
fn test_host_no_context() {
    let result = shim_with_host(":host {}", "contenta", "a-host");
    assert_css_eq!(result, "[a-host] {}");
}

#[test]
fn test_host_with_tag() {
    let result = shim_with_host(":host(ul) {}", "contenta", "a-host");
    assert_css_eq!(result, "ul[a-host] {}");
}

#[test]
fn test_host_with_class() {
    let result = shim_with_host(":host(.x) {}", "contenta", "a-host");
    assert_css_eq!(result, ".x[a-host] {}");
}

#[test]
fn test_host_with_attribute() {
    let result = shim_with_host(":host([a=\"b\"]) {}", "contenta", "a-host");
    assert_css_eq!(result, "[a=\"b\"][a-host] {}");
}

#[test]
fn test_host_with_descendant() {
    let result = shim_with_host(":host .child {}", "contenta", "a-host");
    // :host becomes [a-host], .child gets scoped with [contenta]
    assert!(result.contains("[a-host]"), "Should contain host attr: {result}");
    assert!(result.contains(".child[contenta]"), "Should scope child: {result}");
    assert!(!result.contains(":host"), "Should not contain :host: {result}");
}

// ============================================================================
// ::ng-deep Tests (from ng_deep_spec.ts)
// ============================================================================

#[test]
fn test_ng_deep_removal() {
    let css = "::ng-deep .child { color: blue; }";
    let result = shim(css, "contenta");
    // ::ng-deep should be removed
    assert!(!result.contains("::ng-deep"), "Should remove ::ng-deep");
    assert!(result.contains(".child"), "Should preserve .child");
}

#[test]
fn test_ng_deep_with_host() {
    let css = ":host ::ng-deep .child { color: blue; }";
    let result = shim(css, "contenta");
    // Both :host and ::ng-deep should be handled
    assert!(!result.contains("::ng-deep"), "Should remove ::ng-deep");
    assert!(result.contains(".child"), "Should preserve .child");
}

// ============================================================================
// Complex Selector Tests
// ============================================================================

#[test]
fn test_leave_calc_unchanged() {
    let css = "div {height:calc(100% - 55px);}";
    let expected = "div[contenta] {height:calc(100% - 55px);}";
    assert_css_eq!(shim(css, "contenta"), expected);
}

#[test]
fn test_shim_rules_with_quoted_content() {
    let css = "div {background-image: url(\"a.jpg\"); color: red;}";
    let expected = "div[contenta] {background-image:url(\"a.jpg\"); color:red;}";
    assert_css_eq!(shim(css, "contenta"), expected);
}

#[test]
fn test_handle_curly_braces_in_quoted_content() {
    let css = "div::after { content: \"{}\" }";
    let expected = "div[contenta]::after { content:\"{}\" }";
    assert_css_eq!(shim(css, "contenta"), expected);
}

// ============================================================================
// Playground CSS Test (real-world case)
// ============================================================================

#[test]
fn test_playground_css() {
    // This is from napi/angular-compiler/playground/src/app/app.css
    let css = r"
:host {
  display: block;
  min-height: 100vh;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
}

.app-container {
  padding: 2rem;
  color: white;
}

.app-title {
  font-size: 2.5rem;
  font-weight: bold;
  margin-bottom: 1rem;
}
";

    let result = shim_with_host(css, "contenta", "a-host");

    // :host should be transformed to [a-host]
    assert!(!result.contains(":host"), "Should not contain :host: {result}");
    assert!(result.contains("[a-host]"), "Should contain host attr: {result}");

    // Regular selectors should be scoped with [contenta]
    assert!(result.contains(".app-container[contenta]"), "Should scope .app-container: {result}");
    assert!(result.contains(".app-title[contenta]"), "Should scope .app-title: {result}");

    // CSS properties should be preserved
    assert!(
        result.contains("display: block") || result.contains("display:block"),
        "Should preserve display property"
    );
    assert!(
        result.contains("padding: 2rem") || result.contains("padding:2rem"),
        "Should preserve padding property"
    );
}

// ============================================================================
// Keyframes and Animations Tests (from keyframes_spec.ts)
// ============================================================================

#[test]
fn test_scope_keyframes_rules() {
    let css = "@keyframes foo {0% {transform:translate(-50%) scaleX(0);}}";
    let expected = "@keyframes host-a_foo {0% {transform:translate(-50%) scaleX(0);}}";
    assert_eq!(shim(css, "host-a"), expected);
}

#[test]
fn test_scope_webkit_keyframes_rules() {
    let css = "@-webkit-keyframes foo {0% {-webkit-transform:translate(-50%) scaleX(0);}}";
    let expected =
        "@-webkit-keyframes host-a_foo {0% {-webkit-transform:translate(-50%) scaleX(0);}}";
    assert_css_eq!(shim(css, "host-a"), expected);
}

#[test]
fn test_scope_animations_using_local_keyframes() {
    let css = r"
        button {
            animation: foo 10s ease;
        }
        @keyframes foo {
            0% {
            transform: translate(-50%) scaleX(0);
            }
        }
    ";
    let result = shim(css, "host-a");
    assert!(
        result.contains("animation: host-a_foo 10s ease;")
            || result.contains("animation:host-a_foo 10s ease"),
        "Should scope animation name: {result}"
    );
}

#[test]
fn test_not_scope_animations_using_nonlocal_keyframes() {
    let css = r"
        button {
            animation: foo 10s ease;
        }
    ";
    let result = shim(css, "host-a");
    assert!(
        result.contains("animation: foo 10s ease;") || result.contains("animation:foo 10s ease"),
        "Should NOT scope non-local animation name: {result}"
    );
}

#[test]
fn test_scope_animation_name_using_local_keyframes() {
    let css = r"
        button {
            animation-name: foo;
        }
        @keyframes foo {
            0% {
            transform: translate(-50%) scaleX(0);
            }
        }
    ";
    let result = shim(css, "host-a");
    assert!(
        result.contains("animation-name: host-a_foo;")
            || result.contains("animation-name:host-a_foo"),
        "Should scope animation-name: {result}"
    );
}

// ============================================================================
// Pseudo-function Tests (from shadow_css_spec.ts)
// ============================================================================

#[test]
fn test_where_selector_scoping() {
    let result = shim_with_host(":where(.one) {}", "contenta", "hosta");
    assert_css_eq!(result, ":where(.one[contenta]) {}");
}

#[test]
fn test_where_with_multiple_selectors() {
    let result = shim_with_host(":where(.one, .two) {}", "contenta", "hosta");
    assert_css_eq!(result, ":where(.one[contenta], .two[contenta]) {}");
}

#[test]
fn test_where_with_descendant_selectors() {
    let result = shim_with_host(":where(div.one span.two) {}", "contenta", "hosta");
    assert_css_eq!(result, ":where(div.one[contenta] span.two[contenta]) {}");
}

#[test]
fn test_where_with_host() {
    let result = shim_with_host(":where(:host) {}", "contenta", "hosta");
    assert_css_eq!(result, ":where([hosta]) {}");
}

#[test]
fn test_is_selector_scoping() {
    let result = shim_with_host("div:is(.foo) {}", "contenta", "a-host");
    assert_css_eq!(result, "div[contenta]:is(.foo) {}");
}

#[test]
fn test_is_with_multiple_selectors() {
    let result = shim_with_host(":is(.foo, .bar, .baz) {}", "contenta", "a-host");
    assert_css_eq!(result, ":is(.foo[contenta], .bar[contenta], .baz[contenta]) {}");
}

#[test]
fn test_has_selector_scoping() {
    // :has() places attribute BEFORE :has(), not inside
    let result = shim_with_host("div:has(a) {}", "contenta", "hosta");
    assert_css_eq!(result, "div[contenta]:has(a) {}");
}

#[test]
fn test_has_with_multiple_selectors() {
    let result = shim_with_host(":has(a, b) {}", "contenta", "hosta");
    assert_css_eq!(result, "[contenta]:has(a, b) {}");
}

#[test]
fn test_not_selector() {
    let result = shim_with_host(".header:not(.admin) {}", "contenta", "hosta");
    assert_css_eq!(result, ".header[contenta]:not(.admin) {}");
}

// ============================================================================
// Comment Tests (from shadow_css_spec.ts)
// ============================================================================

#[test]
fn test_replace_comments_with_newline() {
    let result = shim("/* b {c} */ b {c}", "contenta");
    assert_eq!(result, "\n b[contenta] {c}");
}

#[test]
fn test_keep_sourcemapping_url_comments() {
    let result = shim("b {c} /*# sourceMappingURL=data:x */", "contenta");
    assert_eq!(result, "b[contenta] {c} /*# sourceMappingURL=data:x */");
}

#[test]
fn test_handle_adjacent_comments() {
    let result = shim("/* comment 1 */ /* comment 2 */ b {c}", "contenta");
    assert_eq!(result, "\n \n b[contenta] {c}");
}

// ============================================================================
// Additional Attribute Selector Tests
// ============================================================================

#[test]
fn test_attribute_selector_pipe() {
    assert_css_eq!(
        shim("one[attr|=\"value\"] {}", "contenta"),
        "one[attr|=\"value\"][contenta] {}"
    );
}

#[test]
fn test_attribute_selector_tilde() {
    assert_css_eq!(
        shim("one[attr~=\"value\"] {}", "contenta"),
        "one[attr~=\"value\"][contenta] {}"
    );
}

#[test]
fn test_attribute_with_space_in_value() {
    assert_css_eq!(
        shim("one[attr=\"va lue\"] {}", "contenta"),
        "one[attr=\"va lue\"][contenta] {}"
    );
}

#[test]
fn test_is_attribute_selector() {
    assert_css_eq!(shim("[is=\"one\"] {}", "contenta"), "[is=\"one\"][contenta] {}");
}

// ============================================================================
// :host advanced tests
// ============================================================================

#[test]
fn test_host_with_attr_selector() {
    let result = shim_with_host(":host [attr] {}", "contenta", "hosta");
    assert_css_eq!(result, "[hosta] [attr][contenta] {}");
}

#[test]
fn test_host_adjacent_attr() {
    let result = shim_with_host(":host[attr] {}", "contenta", "hosta");
    assert_css_eq!(result, "[attr][hosta] {}");
}

#[test]
fn test_host_with_element() {
    let result = shim_with_host(":host(create-first-project) {}", "contenta", "hosta");
    assert_css_eq!(result, "create-first-project[hosta] {}");
}

// ============================================================================
// :host-context() tests
// ============================================================================

#[test]
fn test_host_context_single() {
    // Single :host-context() should generate two variants
    let result = shim_with_host(":host-context(.dark) {}", "contenta", "hosta");
    // Generates: .dark[hosta], .dark [hosta]
    assert!(result.contains(".dark[hosta]"));
    assert!(result.contains(".dark [hosta]"));
}

#[test]
fn test_host_context_multiple_permutations() {
    // Multiple :host-context() should generate permutations
    let result = shim_with_host(":host-context(.one):host-context(.two) {}", "contenta", "hosta");

    // Should generate 3 permutations x 2 variants = 6 total
    // Compound: .one.two
    assert!(result.contains(".one.two[hosta]"));
    assert!(result.contains(".one.two [hosta]"));

    // .one ancestor of .two
    assert!(result.contains(".one .two[hosta]") || result.contains(".one .two [hosta]"));

    // .two ancestor of .one
    assert!(result.contains(".two .one[hosta]") || result.contains(".two .one [hosta]"));
}

#[test]
fn test_host_context_comma_separated() {
    // Comma-separated selectors inside :host-context()
    let result = shim_with_host(":host-context(.one,.two) .inner {}", "contenta", "hosta");

    // Should generate both .one and .two as separate selectors
    // This should produce something like:
    // .one[hosta] .inner[contenta], .one [hosta] .inner[contenta],
    // .two[hosta] .inner[contenta], .two [hosta] .inner[contenta]
    println!("Result: {result}");

    assert!(result.contains(".one[hosta]") || result.contains(".one [hosta]"));
    assert!(result.contains(".two[hosta]") || result.contains(".two [hosta]"));
}

#[test]
fn test_sidebar_row_layout_css_regression() {
    // This CSS caused a hang in the comparison test at file 1977/5846
    // Testing to identify the problematic pattern
    let css = r"
:host {
  height: var(--sidebar-tree-item-height);
  display: flex;
  align-items: center;
  position: relative;
}

:host:not(.active):has(:is(.row-body:focus-visible, .row-body:hover)) {
  background: var(--hover-background, var(--cu-background-main-hover-strong));
}

:host-context(.sidebar-category-row_editor) :host {
  pointer-events: none;
}

:host-context(.sidebar-everything-row),
:host-context(.sidebar-all-tasks-shared-row),
:host-context(.cu-shared-tasks-row) {
  .row-left {
    padding-left: var(--sidebar-icon-x-spacing);
  }
}

:host-context(.active-hierarchy-item) {
  .everything-row__icon {
    color: var(--cu-content-primary);
  }
}

.row-actions {
  :host-context(.menu-opened-hierarchy-item) &,
  :host-context(.expanded) &,
  :host-context(.tree-node:hover) & {
    visibility: visible;
  }
}
";
    println!("Starting test_sidebar_row_layout_css_regression...");
    let result = shim_with_host(css, "contenta", "hosta");
    println!("Result length: {}", result.len());
    assert!(!result.is_empty());
}

#[test]
fn test_sidebar_row_layout_full_css_regression() {
    // Full CSS from sidebar-row-layout.component.scss that caused a hang
    let css = r"
:host {
  height: var(--sidebar-tree-item-height);
  display: flex;
  align-items: center;
  position: relative;
}

:host:not(.active):has(:is(.row-body:focus-visible, .row-body:hover)) {
  background: var(--hover-background, var(--cu-background-main-hover-strong));
}

:host.expanded .row-toggle-icon {
  transform: rotate(90deg);
}

:host-context(.sidebar-category-row_editor) :host {
  pointer-events: none;
}

:host-context(.sidebar-everything-row),
:host-context(.sidebar-all-tasks-shared-row),
:host-context(.cu-shared-tasks-row) {
  .row-left {
    padding-left: var(--sidebar-icon-x-spacing);
    pointer-events: none;
    position: absolute;
  }
}

:host-context(.sidebar-everything-row),
:host-context(.sidebar-all-tasks-shared-row),
:host-context(.sidebar-create-project-row),
:host-context(.sidebar-shared-row),
:host-context(.cu-shared-tasks-row) {
  .row-avatar {
    font-size: var(--sidebar-icon-dimensions);
    height: var(--sidebar-icon-dimensions);
  }
}

:host-context(.active-hierarchy-item) {
  .everything-row__icon,
  .everything-row__text,
  .shared-row__icon,
  .shared-row__text {
    color: var(--cu-content-primary);
  }

  .row-toggle-icon {
    --button-color: var(--cu-global-sidebar-row-content-active);
  }
}

:host-context(.tree-node:hover .expandable) .row-avatar {
  display: var(--avatar-display, none);
}

:host-context([aria-expanded='true']) :host(.sidebar-with-chat) .row-avatar {
  display: none;
}

:host-context(.sidebar-category-row_editor:hover .expandable) .row-avatar {
  display: flex;
}

.row-left {
  padding-left: var(--sidebar-icon-x-spacing);
  width: var(--sidebar-row-left-width);
}

:host-context(.sidebar-everything-row) .row-left,
:host-context(.sidebar-all-tasks-shared-row) .row-left,
:host-context(.sidebar-shared-row) .row-left,
:host-context(.sidebar-project-row) .row-left {
  padding-left: calc(var(--sidebar-icon-x-spacing) - var(--2px));
}

:host-context(.sidebar-all-projects-row) .row-left {
  display: none;
}

.row-avatar {
  display: flex;
}

.row-toggle-icon {
  /*rtl:raw:
  rotate: 180deg;
  */
}

.row-body {
  flex: 1;
  line-height: var(--sidebar-tree-item-height);
  text-overflow: ellipsis;
  overflow: hidden;
}

:host-context(.active-hierarchy-item) .row-body {
  cursor: default;
}

:host-context(.cdk-drag-preview) .row-body {
  line-height: var(--cu-label-small-line-height);
}

:host-context(.clickable) .row-body {
  cursor: pointer;
}

:host-context(.cu-shared-tasks__inner.home-sidebar) .row-body {
  min-width: 100%;
}

.row-actions {
  display: inline-flex;
  justify-content: flex-end;
  visibility: hidden;
  overflow: clip;
  width: 0;
}

.row-actions:empty {
  margin-inline: revert;
}

:host-context(.menu-opened-hierarchy-item) .row-actions,
:host-context(.expanded) .row-actions,
:host-context(.tree-node:hover) .row-actions,
:host-context(.everything-row) .row-actions,
.row-actions:has(.cu-dropdown_open) {
  visibility: var(--item-visibility, visible);
  overflow: revert;
  width: auto;
  margin-inline: 4px;
}

:host-context(.home-sidebar) :host-context(.menu-opened-hierarchy-item) .row-actions,
:host-context(.home-sidebar) :host-context(.expanded) .row-actions,
:host-context(.home-sidebar) :host-context(.tree-node:hover) .row-actions,
:host-context(.home-sidebar) :host-context(.everything-row) .row-actions,
:host-context(.home-sidebar) .row-actions:has(.cu-dropdown_open) {
  margin-right: 8px;
}

:host-context(.cu-my-work-row) :host-context(.menu-opened-hierarchy-item) .row-actions,
:host-context(.cu-my-work-row) :host-context(.expanded) .row-actions,
:host-context(.cu-my-work-row) :host-context(.tree-node:hover) .row-actions,
:host-context(.cu-my-work-row) :host-context(.everything-row) .row-actions,
:host-context(.cu-my-work-row) .row-actions:has(.cu-dropdown_open) {
  margin-right: 0;
  margin-inline: 0;
}

:host-context(.sidebar-category-row_editor) .row-actions,
:host-context(.sidebar-category-row_editor:hover) .row-actions,
:host-context(.sidebar-category-row_editor .expanded) .row-actions {
  visibility: hidden;
  overflow: clip;
  width: 0;
}

.row-count {
  color: var(--cu-content-tertiary);
  font-size: var(--cu-label-xsmall-font-size);
  margin: 0 var(--cu-size-3) 0 var(--cu-size-2);
}

:host-context(.active-hierarchy-item) .row-count {
  color: var(--cu-global-sidebar-row-content-active, var(--cu-content-primary));
}

:host.highlight-count:not(.home-sidebar) .row-count {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: var(--cu-font-size-2);
  border-radius: var(--cu-radii-round);
  background-color: var(--cu-background-notification);
  color: var(--cu-content-on-dark);
  border: var(--cu-border-size-2) var(--cu-background-subtle) solid;
  height: var(--cu-size-4);
  min-width: var(--cu-size-4);
  padding: 0 var(--3px);
  margin-right: var(--8px);
}

.row-count.highlight {
  margin-right: var(--11px);
}

:host-context(.menu-opened-hierarchy-item) .row-count,
:host-context(.tree-node:hover) .row-count,
:host-context(.tree-node:has(.cu-dropdown_open)) .row-count,
:host-context(.sidebar-category-row_editor) .row-count {
  display: none !important;
}

:host-context(.tree-node:hover) :host.highlight-count .row-count {
  border-color: var(--cu-background-subtle-hover-offset);
}

:host-context(.active-hierarchy-item) :host.highlight-count .row-count,
:host-context(.active-hierarchy-item):hover :host.highlight-count .row-count {
  border-color: var(--cu-background-primary-on-subtle);
}

:host(.home-sidebar) .row-count {
  margin-left: 2px;
}

.row-toggle {
  --button-background-hover: var(--cu-background-on-subtle-hover);
  --button-background-active: transparent;
  display: none;
  margin-left: var(--negative-2px);
  padding-top: var(--2px);
}

:host-context(.show-toggle-always) .row-toggle {
  display: inline-flex;
}

:host-context([aria-expanded='true']) :host(.sidebar-with-chat) .row-toggle {
  display: inline-flex;
  background-color: var(--cu-background-on-subtle-hover);
}

:host-context(.active-hierarchy-item) .row-toggle {
  --button-background-hover: var(--cu-global-sidebar-row-ellipsis-hover, var(--cu-background-primary-on-subtle-hover));
}

:host-context(.sidebar-category-row) .row-toggle {
  margin-left: var(--negative-3px);
}

:host-context(.tree-node:hover) .row-toggle {
  display: var(--button-display, inline-flex);
}

:host-context(.sidebar-category-row_editor:hover) .row-toggle {
  display: none;
}

:host-context(.sidebar-category-row) :host(.home-sidebar) .row-toggle,
:host(.cu-my-work-row__inner.expandable.home-sidebar) .row-toggle {
  margin-left: var(--negative-2px);
}

:host(.home-sidebar) .row-toggle {
  margin-left: 0;
  padding-top: var(--1px);
  position: relative;
}

:host(.home-sidebar) .row-toggle::before {
  content: '';
  position: absolute;
  inset: -4px -8px -4px -7px;
  width: calc(100% + 15px);
  height: calc(100% + 8px);
}
";
    println!("Starting test_sidebar_row_layout_full_css_regression...");
    let result = shim_with_host(css, "contenta", "hosta");
    println!("Result length: {}", result.len());
    assert!(!result.is_empty());
}

// ============================================================================
// Regression: CSS comments before first selector must not break scoping
// ============================================================================

#[test]
fn test_scope_first_selector_after_comment_with_space() {
    // Comment followed by space then selector
    let css = "/* comment */ .foo { color: red; }";
    let expected = ".foo[contenta] { color: red; }";
    assert_css_eq!(shim(css, "contenta"), expected);
}

#[test]
fn test_scope_first_selector_after_comment_with_newline() {
    // Comment followed by newline then selector (the SCSS @import case)
    let css = "/* comment */\n.container { border-radius: 2px; }\n.container .tabs-group { width: 100%; }";
    let expected = ".container[contenta] { border-radius: 2px; }\n.container[contenta] .tabs-group[contenta] { width: 100%; }";
    assert_css_eq!(shim(css, "contenta"), expected);
}

#[test]
fn test_scope_first_selector_after_multiline_comment() {
    // Multi-line comment followed by selector
    let css = "/* multi\nline\ncomment */\n.root { padding: 16px; }\n.root .child { color: red; }";
    let expected =
        ".root[contenta] { padding: 16px; }\n.root[contenta] .child[contenta] { color: red; }";
    assert_css_eq!(shim(css, "contenta"), expected);
}

#[test]
fn test_scope_first_selector_after_multiple_comments() {
    // Multiple comments before first selector
    let css = "/* comment 1 */ /* comment 2 */ .foo { color: red; }";
    let expected = ".foo[contenta] { color: red; }";
    assert_css_eq!(shim(css, "contenta"), expected);
}

#[test]
fn test_newline_as_descendant_combinator() {
    // Newline between selectors is a valid CSS descendant combinator
    let css = ".foo\n.bar { color: red; }";
    let expected = ".foo[contenta] .bar[contenta] { color: red; }";
    assert_css_eq!(shim(css, "contenta"), expected);
}

#[test]
fn test_host_pseudo_with_newline_combinator() {
    // :host with pseudo-selector followed by newline combinator to child
    let css = ":host(:hover)\n.child { color: red; }";
    let expected = "[hosta]:hover .child[contenta] { color: red; }";
    assert_css_eq!(shim_with_host(css, "contenta", "hosta"), expected);
}
