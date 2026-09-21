# Changelog

## 0.0.39 - 2026-09-21

- Fixed JIT mode leaving dead bare imports when every specifier of an import statement is an inline `type` specifier.
- Updated Oxc, napi-rs, and related dependencies.

## 0.0.38 - 2026-08-25

- Fixed template HMR so changes reach every component that shares a template without forcing a full-page reload.
- Fixed style HMR for shared resources, multiple components in one file, imported resources, and components that remove their last style.
- Updated Oxc and related dependencies.
