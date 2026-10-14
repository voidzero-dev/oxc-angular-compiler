# Changelog

## 0.0.41 - 2026-10-14

- Matched Angular 22.2.2 emit: dropped `img|src`/`video|src` from URL sanitization and stripped the namespace on control-flow insertion-point tag names.
- Hoisted template arrow functions through `ɵɵarrowFunction`, and resolved arrow function parameters in template expressions.
- Evaluated decorator `host` and `styles` metadata statically like ngtsc, and resolved decorator metadata imported from other files.
- Reported incomplete control-flow blocks like Angular, and kept capitalised element names as elements.
- Rewrote template HMR updates to write the new template's fields instead of redefining the live definition.
- Reported missing ngtsc diagnostics for inputs, outputs, queries, and signal APIs.
- Deduped generated `i0` namespace imports against all file identifiers.
- Emitted `queries`/`viewQueries` in `ɵɵngDeclareComponent`.
- Lowered all `@angular/core` member decorators into JIT `propDecorators`, prepended synthesized signal-API decorators, and matched ngtsc's `propDecorators` member shape in `setClassMetadata`.
- Ignored `@HostBinding`/`@HostListener` on static members.
- Checked ctor param decorators (arity and unexpected) like `di.ts`, emitted `type: undefined` for `ctorParam` annotations without a runtime value, and downleveled `jit: true` on anonymous classes.
- Kept spread arguments in call/new expressions, defaults/async/rest in metadata arrow functions, and parentheses that JavaScript requires.
- Kept the comments ngtsc keeps on `ngAcceptInputType_*` declaration types.
- Updated Oxc to 0.154, napi-rs, and other dependencies.

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
