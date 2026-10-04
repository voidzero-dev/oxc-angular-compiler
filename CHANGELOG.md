# Changelog

## 0.0.40 - 2026-10-04

- Aligned the DOM sanitization schema with Angular v22: SVG, i18n, and host-binding sinks, plus promoting unknown bare elements for element bindings.
- Compiled `inputs:`, `outputs:`, and `queries:` declared in decorator metadata, and validated input transforms like ngtsc.
- Added `ngAcceptInputType_*` typing from the transform's parameter type.
- Recognized Angular decorators and signal APIs by their `@angular/core` import, matching ngtsc.
- Fixed decorator metadata dropping object methods, accessors, and computed keys.
- Fixed the linker for `ɵ` identifiers written as `\u0275` escapes, template literals in styles and host bindings, and the file's own `@angular/core` namespace in generated templates.
- Fixed incomplete `@let` declarations to report like Angular.
- Ported `specializeControlProperties` for Angular >= 21.2 (Angular 22.2 compat).
- Fixed i18n parity for attribute and ICU messages.
- Fixed template HMR component ids containing `@`.
- Updated Oxc to 0.152, napi-rs, and other dependencies.

## 0.0.39 - 2026-09-21

- Fixed JIT mode leaving dead bare imports when every specifier of an import statement is an inline `type` specifier.
- Updated Oxc, napi-rs, and related dependencies.

## 0.0.38 - 2026-08-25

- Fixed template HMR so changes reach every component that shares a template without forcing a full-page reload.
- Fixed style HMR for shared resources, multiple components in one file, imported resources, and components that remove their last style.
- Updated Oxc and related dependencies.
