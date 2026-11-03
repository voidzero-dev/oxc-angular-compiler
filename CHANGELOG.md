# Changelog

## 0.0.43 - 2026-11-03

- Supported named `deferredImports` blocks (Angular 22.2), and kept native `?.` when linking v22 declarations.
- Defaulted the Vite plugin's `angularVersion` to the installed `@angular/core` version.
- Added `transformForBrowser`: TypeScript with Angular's settings, matching tsc, with async lowered for zone.js.
- Downleveled async/await for zone.js in the Vite plugin like the Angular CLI, and respected package `sideEffects` when marking top-level calls pure.
- Merged compile passes and moved more plugin logic to Rust.
- Closed many ngtsc output gaps: shared long strings and literals in the constant pool, implicit signal `debugName`, the sanitizers added in 22.0.1, dynamic host attribute values, project-wide `preserveWhitespaces`, and legacy i18n message ids.
- Fixed DI tokens: namespace-qualified and same-file interface/type-alias param types, string `@Inject` tokens, `@Attribute()` outside components, flag arrays in `useFactory`/`useClass` deps, and constructor overloads.
- Gave each `@defer` block its own deps function, lazy-loaded `forwardRef(() => X)` imports, deferred imports used only inside `@defer`, and emitted `ɵɵenableIncrementalHydrationRuntime` for incremental hydration.
- Fixed partial output and the linker: NgModule component scope, pipe ctor deps and inherited factories in `ɵɵngDeclareFactory`, escaped metadata strings, `@defer` dependencies, lazy NgModule lists, and declarations in any position.
- Fixed import elision to keep imports the output still uses, and the optimizer's pure wrapping of classes with static members or blocks.
- Read decorator metadata with the partial evaluator, folded number consts in template literals, and wrapped signal query predicates in `forwardRef`.
- Compiled every stacked `@HostListener`/`@HostBinding` on a member, and `animate.enter`/`animate.leave` listeners.
- Fixed parser and lexer cases: a hang on NUL, HTML void names in svg/math, non-ASCII selectors, `<` inside `@let`, escaped strings, and key spans.
- Kept the Vite dev server from serving a stale file while its component fails to compile.
- Updated Oxc to 0.157, napi-rs to 3.16, and other dependencies.

## 0.0.42 - 2026-10-20

- Linked with the application's `angularVersion`, so `img`/`video` `[src]` keeps `ɵɵsanitizeUrl` on runtimes before 22.2.2.
- Resolved template locals inside `$safeNavigationMigration` in listeners and arrow functions.
- Tied closure lifetimes to `Semantic` in validation.
- Updated Oxc to 0.155, keeping string values with lone surrogates exact or treating them as unreadable.
- Updated oxlint and other npm dependencies.

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
