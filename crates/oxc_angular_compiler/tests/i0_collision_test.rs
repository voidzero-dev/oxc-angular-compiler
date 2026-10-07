//! A user binding named `i0` collides with the injected
//! `import * as i0 from "@angular/core"` (issue #509). ngtsc's AOT/dts
//! `ImportManager` (`presetImportManagerForceNamespaceImports`) never reuses
//! an existing namespace import: it mints `i0` and renames it `i0_1`,
//! `i0_2`, ... until it no longer collides with any identifier in the
//! original file (`SourceFile.identifiers`,
//! `check_unique_identifier_name.ts`).

use oxc_allocator::Allocator;
use oxc_angular_compiler::{CompilationMode, TransformOptions, transform_angular_file};

fn compile(source: &str) -> String {
    let allocator = Allocator::default();
    let options =
        TransformOptions { compilation_mode: CompilationMode::Full, ..Default::default() };
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    result.code
}

const COMPONENT: &str = "\
import { Component } from '@angular/core';
@Component({ selector: 'c', template: '<p>hi</p>', standalone: true })
export class C {}
";

#[test]
fn user_const_i0_gets_uniquified_namespace() {
    let out = compile(&format!("const i0 = 1;\n{COMPONENT}"));

    assert!(out.contains("import * as i0_1 from '@angular/core';"), "{out}");
    assert!(out.contains("i0_1.\u{0275}\u{0275}defineComponent"), "{out}");
    assert!(!out.contains("import * as i0 from"), "{out}");
    // The user binding stays untouched.
    assert!(out.contains("const i0 = 1"), "{out}");
}

#[test]
fn user_namespace_import_i0_gets_uniquified_namespace() {
    let out = compile(&format!("import * as i0 from './other';\n{COMPONENT}"));

    assert!(out.contains("import * as i0_1 from '@angular/core';"), "{out}");
    assert!(out.contains("i0_1.\u{0275}\u{0275}defineComponent"), "{out}");
}

#[test]
fn existing_core_namespace_import_is_not_reused() {
    // Upstream's AOT ImportManager never reuses an existing namespace import
    // (`disableOriginalSourceFileReuse`), so `import * as i0` is added next
    // to the user's `import * as ng`, and generated code goes through `i0`.
    // The `setClassMetadata` block keeps referencing `ng.Component`, so the
    // original import survives elision.
    let out = compile(
        "import * as ng from '@angular/core';\n\
         @ng.Component({ selector: 'c', template: '<p>hi</p>', standalone: true })\n\
         export class C {}\n",
    );

    assert!(out.contains("import * as ng from '@angular/core';"), "{out}");
    assert!(out.contains("import * as i0 from '@angular/core';"), "{out}");
    assert!(out.contains("i0.\u{0275}\u{0275}defineComponent"), "{out}");
    assert!(out.contains("type:ng.Component"), "{out}");
}

#[test]
fn unused_core_namespace_import_is_elided() {
    // `ng` is never referenced, so import elision drops it — upstream
    // produces the same, since its transforms run before TypeScript's own
    // import elision.
    let out = compile(
        "import * as ng from '@angular/core';\n\
         import { Component } from '@angular/core';\n\
         @Component({ selector: 'c', template: '', standalone: true })\n\
         export class C {}\n",
    );

    assert!(!out.contains("import * as ng"), "{out}");
    assert!(out.contains("import * as i0 from '@angular/core';"), "{out}");
    assert!(out.contains("i0.\u{0275}\u{0275}defineComponent"), "{out}");
}

#[test]
fn i0_and_i0_1_taken_gives_i0_2() {
    let out = compile(&format!("const i0 = 1;\nconst i0_1 = 2;\n{COMPONENT}"));

    assert!(out.contains("import * as i0_2 from '@angular/core';"), "{out}");
    assert!(out.contains("i0_2.\u{0275}\u{0275}defineComponent"), "{out}");
}

#[test]
fn user_i1_does_not_collide_with_other_module_aliases() {
    let out = compile(
        "import { Component } from '@angular/core';\n\
         import { Router } from '@angular/router';\n\
         const i1 = 0;\n\
         @Component({ selector: 'c', template: '', standalone: true })\n\
         export class C { constructor(r: Router) {} }\n",
    );

    // `i1` is taken, so `@angular/router` gets `i1_1` (the index is still
    // consumed, matching upstream `nextUniqueIndex++`).
    assert!(out.contains("import * as i1_1 from '@angular/router';"), "{out}");
    assert!(out.contains("i1_1.Router"), "{out}");
}

fn dts_members(source: &str) -> String {
    let allocator = Allocator::default();
    let options =
        TransformOptions { compilation_mode: CompilationMode::Full, ..Default::default() };
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "should not have errors, got: {:?}", result.diagnostics);
    result.dts_declarations.iter().map(|d| d.members.as_str()).collect()
}

#[test]
fn dts_members_use_the_dts_file_alias() {
    // `.d.ts` emit mints the same alias the JS emit does: upstream's
    // `IvyDeclarationDtsTransform` runs a second ImportManager over the
    // ORIGINAL file's `SourceFile.identifiers` (declaration.ts), so `i0`
    // stays `i0` — never the user's `ng`.
    let members = dts_members(
        "import * as ng from '@angular/core';\n\
         @ng.Component({ selector: 'c', template: '', standalone: true })\n\
         export class C {}\n",
    );

    assert!(members.contains("i0.\u{0275}\u{0275}ComponentDeclaration"), "{members}");
    assert!(!members.contains("ng."), "{members}");
}

#[test]
fn dts_members_do_not_reuse_a_live_namespace_import() {
    // Even when `ng` survives declaration emit, upstream never reuses it —
    // the dts ImportManager still mints its own `i0`.
    let members = dts_members(
        "import * as ng from '@angular/core';\n\
         export const x: typeof ng.version = ng.version;\n\
         @ng.Component({ selector: 'c', template: '', standalone: true })\n\
         export class C {}\n",
    );

    assert!(members.contains("i0.\u{0275}\u{0275}ComponentDeclaration"), "{members}");
    assert!(!members.contains("ng.\u{0275}"), "{members}");
}

#[test]
fn dts_members_dedupe_against_exported_identifiers() {
    // An exported `i0` survives declaration emit, so the `.d.ts` namespace
    // is uniquified to `i0_1` independently of the JS emit's choice.
    let members = dts_members(&format!("export const i0 = 1;\n{COMPONENT}"));

    assert!(members.contains("i0_1.\u{0275}\u{0275}ComponentDeclaration"), "{members}");
}

#[test]
fn dts_members_dedupe_against_all_identifiers_regardless_of_exports() {
    // `import * as i0` puts `i0` in the original file's identifier set even
    // where it is only named in a type position — the members use `i0_1`,
    // and a dropped `interface U` makes no difference.
    let members = dts_members(
        "import * as i0 from './other';\n\
         import { Component } from '@angular/core';\n\
         interface U { x: i0.Type }\n\
         @Component({ selector: 'c', template: '<p>hi</p>', standalone: true })\n\
         class C { field!: i0.Type }\n\
         export { C };\n",
    );

    assert!(members.contains("i0_1.\u{0275}\u{0275}ComponentDeclaration"), "{members}");
    assert!(members.contains("i0_1.\u{0275}\u{0275}FactoryDeclaration"), "{members}");
}

#[test]
fn dts_members_dedupe_against_all_file_identifiers() {
    // An unexported `const i0` still appears in the original file's
    // `SourceFile.identifiers`, so upstream's dts ImportManager picks
    // `i0_1` — identical to the JS emit's choice.
    let members = dts_members(&format!("const i0 = 1;\n{COMPONENT}"));

    assert!(members.contains("i0_1.\u{0275}\u{0275}ComponentDeclaration"), "{members}");
    assert!(!members.contains("i0.\u{0275}"), "{members}");
}

#[test]
fn dts_input_transform_type_uses_the_uniquified_alias() {
    // `ngAcceptInputType_*` types print `@angular/core` names through the
    // same uniquified alias as the other members — a user `i0` must not
    // capture `i0.Signal`.
    let source = "\
import { Component, Input, Signal } from '@angular/core';
const i0 = 1;
@Component({ selector: 'c', template: '', standalone: true })
export class C {
  @Input({ transform: (v: Signal<number>) => v }) t: any;
}
";
    let allocator = Allocator::default();
    let options =
        TransformOptions { compilation_mode: CompilationMode::Full, ..Default::default() };
    let result = transform_angular_file(&allocator, "test.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    let members: String = result.dts_declarations.iter().map(|d| d.members.as_str()).collect();

    assert!(members.contains("ngAcceptInputType_t: i0_1.Signal<number>"), "{members}");
    assert!(!members.contains("i0.Signal"), "{members}");
    // The alias's module is recorded so the consumer emits its import.
    assert_eq!(
        result.dts_declarations[0].namespace_imports.get("i0_1").map(String::as_str),
        Some("@angular/core"),
        "{:?}",
        result.dts_declarations[0].namespace_imports
    );
}

#[test]
fn user_i0_still_resolves_in_metadata() {
    // A user value named `i0` referenced inside decorator metadata must keep
    // resolving to the user binding, not the generated namespace.
    let out = compile(
        "import { Component } from '@angular/core';\n\
         const i0 = { provide: 'x' };\n\
         @Component({ selector: 'c', template: '', standalone: true, providers: [i0] })\n\
         export class C {}\n",
    );

    assert!(out.contains("import * as i0_1 from '@angular/core';"), "{out}");
    // `providers: [i0]` in the emitted definition still reads the user const.
    assert!(out.replace(char::is_whitespace, "").contains("providers:[i0]"), "{out}");
}
