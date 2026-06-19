# Upstream Angular Compiler — Drift & Gap Report

**Generated:** 2026-06-19
**Pinned submodule:** `crates/oxc_angular_compiler/angular` @ `1cb0524f82ebcc06642ceebd0b96a19bba883b2e`
**Upstream HEAD at time of report:** `302dd0f7c606c3fefede7c2e4b815f8579cc5b28`

---

## 1. Summary

| | Tag | Commit | Date |
|---|---|---|---|
| **Pinned** | `v22.0.0` | `1cb0524` | 2026-06-03 |
| **Latest** | `v22.1.0-next.1` | `302dd0f` | 2026-06-17 |

Intermediate releases not yet picked up: `v22.0.0-rc.3`, `v22.0.1`, `v22.0.2`, `v22.1.0-next.0`.

In the drift window, upstream landed **40 commits touching `packages/compiler/`** and **49 touching
`packages/compiler-cli/`**. The changes cluster into three themes:

1. **Foreign components / `@content` blocks** — a large new template feature (dominant theme).
2. **Security / sanitization hardening** — several correctness fixes, some marked security-sensitive.
3. **In-element comments** — the ML lexer/parser now retains comments written inside element tags.

Plus smaller refactors (emitter, regex optimization, type-check for-loops, isolatedDeclarations support).

### Documentation drift (fix this first — it's misleading)

- The submodule is pinned to **`v22.0.0`** (the final release), but
  `crates/angular_conformance/README.md` line 7 claims conformance runs against **`v22.0.0-rc.2`**.
  The README is wrong about the pinned version. (`v22.0.0-rc.2` = `ebd698b`, not `1cb0524`.)

---

## 2. Theme A — Foreign components & `@content` blocks (NOT IMPLEMENTED)

This is the single biggest source of drift. Upstream introduced a new "foreign component" / content-
projection mechanism with a brand-new `@content` template block. It spans `compiler`, `compiler-cli`,
and `core`.

Key upstream commits (`packages/compiler`):

- `f89d0e4` support passing children to foreign components
- `f19bbe5` support passing content to specific foreign component props
- `b399f78` support passing `@content` blocks as functions
- `6981cd7` emit instructions for foreign components
- `88b0e42` add support for importing foreign components
- `d596d8b` support matching and validating foreign components in templates (#68674)
- `25c744c` support foreign components defined outside top-level scope
- `1f6e843` / `1f6e843` validate `@content` block placement (compiler-cli)

New source artifacts upstream (none of which exist in the Rust port):

- `packages/compiler/src/render3/r3_content_blocks.ts` (new, 142 lines)
- New `ContentBlock` node + `visitContentBlock` visitor in `render3/r3_ast.ts`
- `render3/view/t2_api.ts` → `ForeignComponentMeta`, `getForeignComponent`
- `template/pipeline/src/phases/resolve_foreign_content.ts` (new phase)
- New IR ops/enums/expressions for foreign content
- `r3_identifiers.ts` → `ɵɵforeignComponent` instruction

New upstream conformance tests that the Rust port has no fixtures/coverage for:

- `render3/r3_template_transform_spec.ts` → new `describe('@content blocks')` (7 tests: valid block,
  invalid name, missing/extra parameters, variables, variable-with-value error, invalid variable name).
- `render3/r3_ast_spans_spec.ts` → `visitContentBlock` span humanization.
- `render3/view/binding_spec.ts` → "should match foreign components by tag name" and the
  directive/foreign-component conflict error.

**Gap in the Rust port (verified):**

- `crates/oxc_angular_compiler/src/parser/html/lexer.rs` — the `SUPPORTED_BLOCKS` list ends at
  `"error"`; **`"content"` is missing**. Upstream added `'@content'` to `SUPPORTED_BLOCKS` in
  `ml_parser/lexer.ts`.
- No `ContentBlock` AST node anywhere (`grep -ri content_block src` → 0 hits).
- No foreign-component matching/binding in the t2 binder equivalent.

**Recommendation:** This is a feature-sized effort, not a quick port. Track as a dedicated work item.
The template-compiler-relevant slice (lexing `@content`, the `ContentBlock` R3 node, transform +
ingest + a `resolve_foreign_content` phase + the `ɵɵforeignComponent` instruction) is in scope for
this project; the `compiler-cli` validation/type-check half is mostly out of scope.

---

## 3. Theme B — Security & sanitization (PARTIAL — review needed)

Several upstream fixes are security-sensitive. The Rust port already carries an explicit MathML URL
enumeration and an `AttributeNoBinding` `SecurityContext` variant
(`src/ast/r3.rs:630`, `src/schema/dom_security_schema.rs`), so it is *partially* ahead of its own pin
in this area — but it does **not** match upstream's refactored behaviour:

| Upstream change | Commit | Rust port status |
|---|---|---|
| iframe `credentialless` → `ATTRIBUTE_NO_BINDING` | `0152e3c` | **Missing** (`grep -ri credentialless src` → 0 hits) |
| Sanitize `href`/`xlink:href` on **any** MathML-namespace element (wildcard) | `3927a5b` | **Behaviourally different** — port enumerates a fixed MathML element list instead of a namespace wildcard; an unknown MathML tag like `:math:foobar` would not match |
| Normalize tag names with custom namespaces in `DomElementSchemaRegistry` | `4d79a52`, `61a97f2` | Not modeled — port's `get_security_context(element, property)` lowercases a raw tag name and has no `:math:` / `:svg:` namespace normalization |
| Sanitize dynamic `href`/`xlink:href` on SVG `<a>` | `75033d2` | Enumerated entries present; runtime-binding nuance not verified |
| Sanitize two-way properties | `3c70270` | Needs review in `resolve_sanitizers` phase |
| Disallow i18n event attributes | `6c41f5c` | Needs review |
| Strip namespaced SVG `<script>`/`<style>` during compilation | `90494cd`, `ec138c3` | Needs review |
| Disallow event attribute bindings in host bindings unconditionally | `5b421c6` | Needs review |

New upstream tests (in `schema/dom_element_schema_registry_spec.ts`) assert all of the above —
`:math:math`, `:math:foobar` (wildcard), `:svg:foobar` (must stay `NONE`), `iframe credentialless`
→ `ATTRIBUTE_NO_BINDING`, and `p|href` → `NONE`.

**Coverage gap (pre-existing, but newly relevant):** the conformance harness extracts
`schema_dom_element_schema_registry_spec.json` and `schema_trusted_types_sinks_spec.json` but
**registers no runner for them** (`crates/angular_conformance/src/subsystems/mod.rs` registers 11
runners; no schema runner). So none of these security assertions are exercised by conformance today.

**Recommendation:** Treat the iframe-`credentialless` and MathML-wildcard items as security follow-ups.
Add a `DomElementSchemaRegistry` security runner so these assertions are actually checked.

---

## 4. Theme C — In-element comments (NOT IMPLEMENTED)

Commit `471dcb4` ("Collect in-element comments") changes the ML lexer to **retain** comments written
inside an element's open tag (`//` and `/* */`) instead of silently discarding them:

- `ml_parser/tokens.ts` — new `IN_ELEMENT_COMMENT` token type + `InElementCommentToken`.
- `ml_parser/lexer.ts` — `_consumeSingleLineComment` / `_consumeMultiLineComment` now emit a token.
- `ml_parser/parser.ts` — in-element comment tokens become `html.Comment` nodes on the parent.
- `parseTemplate(..., {collectCommentNodes: true})` now surfaces them.

New tests: `render3/view/parse_template_options_spec.ts` — `collectCommentNodes` count goes 2 → 5,
asserting source spans for `//`, `/* */`, and trailing-space comment variants inside a tag.

**Gap in the Rust port (verified):** no `IN_ELEMENT_COMMENT` token (`grep -ri in_element_comment src`
→ 0 hits). The Rust lexer (`src/parser/html/lexer.rs`) still consumes/discards in-tag comments.

Note: upstream did **not** modify `lexer_spec.ts` / `html_parser_spec.ts` for this — coverage rides
on `parse_template_options_spec`, which the port does not appear to run as a subsystem either.

---

## 5. Theme D — Smaller compiler changes worth porting

| Change | Commit | Affected Rust area |
|---|---|---|
| Remove 80-char line limit in `AbstractEmitterVisitor` | `54112d9` | `src/output/emitter.rs` — check for any hard-wrapping at 80 cols; output may now differ |
| More robust "can this regex be optimized" check | `636cc94` | `regular_expression_optimization` pipeline phase |
| Move projection attributes into constants | `f0b28f6` | r3 view compiler / const pool |
| Type-check invalid `@for` loops | `06f6dec` | Mostly compiler-cli (type-check) — likely out of scope |
| Preserve leading commas in animation definitions | `770e505` | style/animation parsing |
| `:host-context` requires parentheses w/ args; drop legacy shadow-DOM selectors & polyfills | `338bf7d`, `1d3bf59`, `6de4955`, `23f0898` | `shadow_css` — verify against `shadow_css_*` fixtures |
| isolatedDeclarations support (NgModules, host directives, `@Input` transforms) | `06b004e` et al. | compiler-cli — out of scope for this port |

The `shadow_css` items are the most likely to affect this port's existing 169 `shadow_css` conformance
assertions if the submodule is bumped.

---

## 6. Conformance fixture staleness

Because fixtures are extracted from the pinned submodule, bumping the pin and re-running
`cargo run -p oxc_angular_conformance -- --generate` will pull in the new upstream tests above. Files
that will change on regeneration:

- `render3_r3_template_transform_spec.json` (+`@content` block tests)
- `render3_r3_ast_spans_spec.json` (+`ContentBlock` spans)
- `render3_view_binding_spec.json` (+foreign-component matching)
- `render3_view_parse_template_options_spec.json` (+in-element comments)
- `schema_dom_element_schema_registry_spec.json` (+MathML/iframe security) — *currently has no runner*

Expect the **100% / 1264-assertion** pass rate to regress once regenerated, until the gaps above are
closed. The new `@content` and in-element-comment tests will fail; foreign-component binding tests have
no runner; the security tests have no runner.

---

## 7. Recommended next steps (in priority order)

1. **Doc fix (trivial):** correct the `v22.0.0-rc.2` claim in `crates/angular_conformance/README.md`
   to `v22.0.0` (the actual pin).
2. **Security follow-ups (high value, small):** add iframe `credentialless` → `AttributeNoBinding`;
   make MathML `href`/`xlink:href` a namespace wildcard rather than an enumerated list; add a
   `DomElementSchemaRegistry` conformance runner so these are actually tested.
3. **In-element comments (medium):** add `IN_ELEMENT_COMMENT` lexing + parser `Comment` emission;
   wire `collectCommentNodes`.
4. **`@content` / foreign components (large, feature-sized):** scope the template-compiler slice as a
   dedicated milestone; defer/exclude the compiler-cli type-check half.
5. **Bump the submodule pin** to a chosen target (suggest the stable `v22.0.2`, or `v22.1.0-next.1`
   to match upstream HEAD), regenerate fixtures, and triage the resulting conformance diff.

---

## 8. How this was produced (reproducible)

```bash
# Blobless clone of upstream for fast diffing
git clone --filter=blob:none --no-checkout --bare https://github.com/angular/angular.git ng
cd ng
PIN=1cb0524f82ebcc06642ceebd0b96a19bba883b2e   # v22.0.0 (current submodule pin)
LATEST=302dd0f7c606c3fefede7c2e4b815f8579cc5b28 # v22.1.0-next.1 (upstream HEAD)

git log --no-merges --oneline $PIN..$LATEST -- packages/compiler
git diff --stat $PIN..$LATEST -- packages/compiler/src
git diff --stat $PIN..$LATEST -- packages/compiler/test
```
