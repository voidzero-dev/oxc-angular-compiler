//! Regression tests for issue #520: `ɵsetClassMetadata`'s `ctorParameters`
//! callback names ctor-parameter decorators (`{type: Optional}`), `@Inject`
//! tokens (`args: [TOKEN]`), and member decorators (`{type: Input}`) as bare
//! identifiers. Import elision must keep those imports whenever metadata is
//! emitted (the default), like ngtsc — otherwise `TestBed.overrideComponent`
//! and friends throw `ReferenceError` when they invoke `ctorParameters`.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, transform_angular_file};

fn compile(source: &str) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "test.ts", source, None, None);
    assert!(!result.has_errors(), "unexpected errors: {:?}", result.diagnostics);
    result.code
}

fn compile_with(source: &str, options: &TransformOptions) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "test.ts", source, Some(options), None);
    assert!(!result.has_errors(), "unexpected errors: {:?}", result.diagnostics);
    result.code
}

/// Slice out the `ctorParameters`/`propDecorators` metadata argument text so a
/// test can assert about the names the callback references.
fn metadata_block(code: &str) -> &str {
    let start = code.find("setClassMetadata").expect("setClassMetadata missing");
    &code[start..]
}

/// The issue's exact repro: `@Optional() @Inject(TOKEN)` on a @Component ctor
/// parameter must keep `Inject`, `Optional`, and `TOKEN` imported.
#[test]
fn ctor_param_decorators_keep_imports_with_class_metadata() {
    let source = r#"
import {Component, Inject, Optional} from '@angular/core';
import {TOKEN} from './tokens';
@Component({selector: 'c', template: ''})
export class C {
  constructor(@Optional() @Inject(TOKEN) x: unknown) {}
}
"#;
    let code = compile(source);

    assert!(
        code.contains("import {Component, Inject, Optional}"),
        "Inject/Optional imports must survive: {code}"
    );
    assert!(code.contains("import {TOKEN} from './tokens'"), "TOKEN import must survive: {code}");

    // The metadata still names them bare — now bound by the kept imports.
    let metadata = metadata_block(&code);
    assert!(metadata.contains("{type:Optional}"), "{metadata}");
    assert!(metadata.contains("{type:Inject,args:[TOKEN]}"), "{metadata}");
}

/// Same repro with `emit_class_metadata` off: the decorators are stripped and
/// their imports are elided, as before.
#[test]
fn ctor_param_decorators_elided_without_class_metadata() {
    let source = r#"
import {Component, Inject, Optional} from '@angular/core';
import {TOKEN} from './tokens';
@Component({selector: 'c', template: ''})
export class C {
  constructor(@Optional() @Inject(TOKEN) x: unknown) {}
}
"#;
    let options = TransformOptions { emit_class_metadata: false, ..TransformOptions::default() };
    let code = compile_with(source, &options);

    assert!(!code.contains("setClassMetadata"), "{code}");
    let core_import = code
        .lines()
        .find(|l| l.starts_with("import") && l.contains("@angular/core") && !l.contains("* as"))
        .unwrap();
    assert_eq!(core_import, "import { Component } from \"@angular/core\";");
    assert!(!code.contains("import {TOKEN}"), "TOKEN should be elided: {code}");
}

/// `@Inject(TOKEN)` must also keep the decorator imports for @Directive,
/// @Injectable, @Pipe, and @NgModule classes — they all get setClassMetadata.
#[test]
fn ctor_param_decorators_kept_for_all_decorated_kinds() {
    let source = r#"
import {Directive, Injectable, Inject, Optional} from '@angular/core';
import {TOKEN} from './tokens';
@Directive({selector: '[d]'})
export class D {
  constructor(@Optional() @Inject(TOKEN) x: unknown) {}
}
@Injectable()
export class S {
  constructor(@Inject(TOKEN) y: unknown) {}
}
"#;
    let code = compile(source);

    assert!(code.contains("Inject"), "Inject import must survive: {code}");
    assert!(code.contains("Optional"), "Optional import must survive: {code}");
    assert!(code.contains("import {TOKEN} from './tokens'"), "TOKEN import must survive: {code}");
    assert_eq!(code.matches("setClassMetadata").count(), 2, "{code}");
}

/// `import { Inject as Inj }`: the metadata emits the local name `{type: Inj}`,
/// so the aliased specifier must be kept (it already was — regression guard).
#[test]
fn aliased_param_decorators_keep_imports() {
    let source = r#"
import {Component, Inject as Inj, Optional as Opt} from '@angular/core';
import {TOKEN} from './tokens';
@Component({selector: 'c', template: ''})
export class C {
  constructor(@Opt() @Inj(TOKEN) x: unknown) {}
}
"#;
    let code = compile(source);

    assert!(
        code.contains("import {Component, Inject as Inj, Optional as Opt}"),
        "aliased imports must survive: {code}"
    );
    assert!(code.contains("import {TOKEN} from './tokens'"), "{code}");
    let metadata = metadata_block(&code);
    assert!(metadata.contains("{type:Opt}"), "{metadata}");
    assert!(metadata.contains("{type:Inj,args:[TOKEN]}"), "{metadata}");
}

/// `@Attribute(ATTR)`: the decorator name dangled before the fix even though
/// its argument's import was kept by semantic analysis.
#[test]
fn attribute_decorator_keeps_import() {
    let source = r#"
import {Component, Attribute} from '@angular/core';
import {ATTR} from './tokens';
@Component({selector: 'c', template: ''})
export class C {
  constructor(@Attribute(ATTR) x: string) {}
}
"#;
    let code = compile(source);

    let core_import = code
        .lines()
        .find(|l| l.starts_with("import") && l.contains("@angular/core") && !l.contains("* as"))
        .unwrap();
    assert!(core_import.contains("Attribute"), "Attribute must stay imported: {code}");
    let metadata = metadata_block(&code);
    assert!(metadata.contains("type:Attribute"), "{metadata}");
    assert!(metadata.contains("args:[ATTR]"), "{metadata}");
}

/// A token declared in the same file needs no import, but the param decorators
/// still name `Inject`/`Optional` bare.
#[test]
fn local_token_keeps_decorator_imports() {
    let source = r#"
import {Component, Inject, Optional, InjectionToken} from '@angular/core';
const TOKEN = new InjectionToken<unknown>('TOKEN');
@Component({selector: 'c', template: ''})
export class C {
  constructor(@Optional() @Inject(TOKEN) x: unknown) {}
}
"#;
    let code = compile(source);

    let core_import = code
        .lines()
        .find(|l| l.starts_with("import") && l.contains("@angular/core") && !l.contains("* as"))
        .unwrap();
    assert!(core_import.contains("Inject"), "{code}");
    assert!(core_import.contains("Optional"), "{code}");
}

/// A member decorator on a `declare`d field is still emitted into
/// `propDecorators` (`{x: [{type: Input}]}`), so its import must survive too.
#[test]
fn declare_prop_decorator_keeps_import() {
    let source = r#"
import {Component, Input} from '@angular/core';
@Component({selector: 'c', template: ''})
export class C {
  @Input() declare x: string;
}
"#;
    let code = compile(source);

    let core_import = code
        .lines()
        .find(|l| l.starts_with("import") && l.contains("@angular/core") && !l.contains("* as"))
        .unwrap();
    assert!(core_import.contains("Input"), "Input must stay imported: {code}");
    let metadata = metadata_block(&code);
    assert!(metadata.contains("{x:[{type:Input}]}"), "{metadata}");
}

/// Param decorators on an *undecorated* class are left in the emitted source,
/// so their imports must survive regardless of class metadata.
#[test]
fn param_decorators_on_undecorated_class_keep_imports() {
    let source = r#"
import {Inject, Optional} from '@angular/core';
export class NotDecorated {
  constructor(@Optional() @Inject(String) x: unknown) {}
}
"#;
    let code = compile(source);

    assert!(code.contains("import {Inject, Optional}"), "{code}");
    assert!(
        code.contains("@Optional() @Inject(String)"),
        "decorators should be left in place: {code}"
    );
}

/// With no class metadata AND advanced optimizations, elision still applies.
#[test]
fn ctor_param_decorators_elided_with_advanced_optimizations() {
    let source = r#"
import {Component, Inject, Optional} from '@angular/core';
import {TOKEN} from './tokens';
@Component({selector: 'c', template: ''})
export class C {
  constructor(@Optional() @Inject(TOKEN) x: unknown) {}
}
"#;
    let options = TransformOptions { advanced_optimizations: true, ..TransformOptions::default() };
    let code = compile_with(source, &options);

    assert!(!code.contains("setClassMetadata"), "{code}");
    let core_import = code
        .lines()
        .find(|l| l.starts_with("import") && l.contains("@angular/core") && !l.contains("* as"))
        .unwrap();
    assert!(!core_import.contains("Inject"), "Inject should be elided: {code}");
    assert!(!core_import.contains("Optional"), "Optional should be elided: {code}");
    assert!(!code.contains("import {TOKEN}"), "TOKEN should be elided: {code}");
}
