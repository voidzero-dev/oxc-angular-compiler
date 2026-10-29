//! ngtsc compiles a decorated class declared in a function body, block or
//! callback like a top-level one: its definitions go on the class and its
//! metadata calls right after it, in the same scope, while the constants and
//! child template functions its compile declares go to the file top level,
//! in the file's one constant pool.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    CompilationMode, ComponentHmrUpdateInput, ResolvedResources, TransformOptions, TransformResult,
    compile_component_hmr_update, extract_component_resource_urls, transform_angular_file,
};

fn transform_with(source: &str, options: &TransformOptions) -> TransformResult {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "test.ts", source, Some(options), None);
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    result
}

fn transform(source: &str) -> String {
    transform_with(source, &TransformOptions::default()).code
}

/// `code` without whitespace, so the emitter's line wrapping doesn't matter.
fn squash(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The names of the top-level `const`s of `code`, in order.
fn constants(code: &str) -> Vec<&str> {
    code.lines()
        .filter_map(|line| line.strip_prefix("const "))
        .filter_map(|rest| rest.split([' ', '=']).next())
        .collect()
}

/// The byte offset of `needle` in `code`.
fn offset(code: &str, needle: &str) -> usize {
    code.find(needle).unwrap_or_else(|| panic!("no {needle:?} in\n{code}"))
}

const IIFE: &str = r"
import { Component } from '@angular/core';
export const X = (() => {
  @Component({ selector: '[a]', template: '<ng-content></ng-content>', standalone: true })
  class Inner {}
  return Inner;
})();
@Component({ selector: '[a]', template: '<ng-content></ng-content>', standalone: true })
export class Outer {}
";

#[test]
fn compiles_a_component_declared_in_an_iife() {
    // ngtsc: `_c0` is declared once, after the imports, and both classes use
    // it; Inner's metadata calls stay in the arrow, before `return Inner`.
    let code = transform(IIFE);
    let squashed = squash(&code);
    assert!(!code.contains("@Component("), "{code}");
    assert_eq!(constants(&code), ["_c0"], "{code}");
    assert!(offset(&code, "const _c0") < offset(&code, "export const X"), "{code}");
    assert!(
        squashed.contains("classInner{staticɵfac=functionInner_Factory("),
        "Inner gets its definitions:\n{code}"
    );
    assert!(squashed.contains("type:Inner,selectors:[[\"\",\"a\",\"\"]],ngContentSelectors:_c0"));
    assert!(squashed.contains("type:Outer,selectors:[[\"\",\"a\",\"\"]],ngContentSelectors:_c0"));
    let metadata = offset(&code, "ɵsetClassMetadata(Inner");
    assert!(metadata < offset(&code, "return Inner;"), "{code}");
    assert!(offset(&code, "ɵsetClassDebugInfo(Inner") < offset(&code, "return Inner;"), "{code}");
}

#[test]
fn compiles_each_kind_of_decorated_class_in_a_function_block_or_callback() {
    let code = transform(
        r"
import { Component, Directive, Pipe, Injectable, NgModule } from '@angular/core';
declare const describe: any;
export function make() {
  @Directive({ selector: '[fd]', standalone: true })
  class FnDir {}
  @Pipe({ name: 'fp', standalone: true })
  class FnPipe { transform(v: any) { return v; } }
  @Injectable()
  class FnSvc {}
  @NgModule({})
  class FnMod {}
  return [FnDir, FnPipe, FnSvc, FnMod];
}
{
  @Component({ selector: 'blk', template: '<p>hi</p>', standalone: true })
  class BlockCmp {}
}
describe('x', () => {
  @Component({ selector: 'cb', template: '<p>cb</p>', standalone: true })
  class CallbackCmp {}
});
",
    );
    for decorator in ["@Component(", "@Directive(", "@Pipe(", "@Injectable(", "@NgModule("] {
        assert!(!code.contains(decorator), "{code}");
    }
    let squashed = squash(&code);
    for definition in [
        "staticɵdir=/*@__PURE__*/i0.ɵɵdefineDirective({type:FnDir",
        "staticɵpipe=/*@__PURE__*/i0.ɵɵdefinePipe({name:\"fp\",type:FnPipe",
        "staticɵprov=/*@__PURE__*/i0.ɵɵdefineInjectable({token:FnSvc",
        "staticɵmod=/*@__PURE__*/i0.ɵɵdefineNgModule({type:FnMod",
        "staticɵcmp=/*@__PURE__*/i0.ɵɵdefineComponent({type:BlockCmp",
        "staticɵcmp=/*@__PURE__*/i0.ɵɵdefineComponent({type:CallbackCmp",
    ] {
        assert!(squashed.contains(definition), "no {definition} in\n{code}");
    }
}

#[test]
fn classes_of_one_name_in_two_functions_keep_their_own_definitions() {
    // ngtsc names the second class's template functions `Tp_Template1` and
    // `Tp_Conditional_0_Template1`, all declared at the file top level.
    let code = transform(
        r"
import { Component } from '@angular/core';
export function mk() {
  @Component({ selector: 'tp', template: '@if (a) {<i>{{a}}</i>}', standalone: true })
  class Tp { a = 1; }
  return Tp;
}
export function mk2() {
  @Component({ selector: 'tp2', template: '@if (a) {<b>{{a}}</b>}', standalone: true })
  class Tp { a = 1; }
  return Tp;
}
",
    );
    let squashed = squash(&code);
    assert_eq!(squashed.matches("selectors:[[\"tp\"]]").count(), 1, "{code}");
    assert_eq!(squashed.matches("selectors:[[\"tp2\"]]").count(), 1, "{code}");
    assert!(code.contains("\nfunction Tp_Conditional_0_Template(rf,ctx)"), "{code}");
    assert!(code.contains("\nfunction Tp_Conditional_0_Template1(rf,ctx)"), "{code}");
    assert!(
        squashed.contains("selectors:[[\"tp2\"]],decls:1,vars:1,template:functionTp_Template1("),
        "{code}"
    );
    assert!(offset(&code, "Tp_Conditional_0_Template1(rf") < offset(&code, "function mk2"));
}

#[test]
fn compiles_a_class_in_a_member_before_the_class_and_declares_its_constants_first() {
    // ngtsc compiles M before O: M pools `_c0` and `_c1`, O `_c2`. All go
    // before `export class O`, past the decorator compilation removes.
    let code = transform(
        r"
import { Component } from '@angular/core';
@Component({ selector: 'o', template: '<ng-content></ng-content>', standalone: true })
export class O {
  make() {
    @Component({ selector: 'm', template: '<ng-content select=b></ng-content>', standalone: true })
    class M {}
    return M;
  }
}
",
    );
    assert_eq!(constants(&code), ["_c0", "_c1", "_c2"], "{code}");
    assert!(offset(&code, "const _c2") < offset(&code, "export class O"), "{code}");
    assert!(!code.contains("@Component("), "{code}");
    let squashed = squash(&code);
    assert!(squashed.contains("type:M,selectors:[[\"m\"]],ngContentSelectors:_c1"), "{code}");
    assert!(squashed.contains("i0.ɵɵprojectionDef(_c0)"), "{code}");
    assert!(squashed.contains("type:O,selectors:[[\"o\"]],ngContentSelectors:_c2"), "{code}");
}

#[test]
fn keeps_the_decorators_of_a_nested_class_reading_an_enclosing_binding() {
    // The decorator evaluator only sees top-level bindings: ngtsc folds the
    // local `sel` ("local") and `tpl`; this class stays for the JIT compiler
    // rather than be compiled with the top-level `sel`.
    let source = r"
import { Component } from '@angular/core';
const sel = 'top';
export function f() {
  const sel = 'local';
  @Component({ selector: sel, template: '', standalone: true })
  class Shadowed {}
  const tpl = '<b>x</b>';
  @Component({ selector: 'lr', template: tpl, standalone: true })
  class LocalTemplate {}
  return [Shadowed, LocalTemplate];
}
";
    let code = transform(source);
    assert_eq!(code, source);
}

#[test]
fn keeps_the_decorators_of_a_nested_class_injecting_a_shadowing_local_class() {
    // ngtsc injects the local LSvc; the import map only knows the imported
    // one, so A stays for the JIT compiler. The other classes compile.
    let source = r"
import { Component, Injectable } from '@angular/core';
import { LSvc } from './svc';
export const G = LSvc;
export function f() {
  @Injectable() class LSvc {}
  @Injectable() class Own {}
  @Component({ selector: 'x', template: '', standalone: true, providers: [Own] })
  class A { constructor(s: LSvc, o: Own) {} }
  return A;
}
";
    for compilation_mode in [CompilationMode::Full, CompilationMode::Partial] {
        let code =
            transform_with(source, &TransformOptions { compilation_mode, ..Default::default() })
                .code;
        assert!(!code.contains("i1.LSvc"), "{code}");
        assert!(code.contains("@Component({ selector: 'x'"), "{code}");
        assert!(!code.contains("@Injectable()"), "{code}");
    }
}

#[test]
fn declares_the_constants_of_a_nested_class_after_the_imports() {
    // `make` runs before its statement: ngtsc declares `_c0` after the
    // imports, so `M`'s `ngContentSelectors` doesn't read it in its TDZ.
    let code = transform(
        r"
import { Component } from '@angular/core';
export const Made = make();
function make() {
  @Component({ selector: 'm', template: '<ng-content></ng-content>', standalone: true })
  class M {}
  return M;
}
",
    );
    assert_eq!(constants(&code), ["_c0"], "{code}");
    assert!(offset(&code, "import * as i0") < offset(&code, "const _c0"), "{code}");
    assert!(offset(&code, "const _c0") < offset(&code, "export const Made"), "{code}");
}

#[test]
fn declares_a_constant_a_nested_class_shares_with_an_earlier_class_after_the_imports() {
    // `B` reuses the `_c0` that `A` pooled first; `make` runs before `A`'s
    // statement, so `_c0` goes after the imports, as in ngtsc.
    let code = transform(
        r"
import { Component } from '@angular/core';
export const Made = make();
@Component({ selector: 'a-a', template: '<ng-content></ng-content>' })
export class A {}
function make() {
  @Component({ selector: 'b-b', template: '<ng-content></ng-content>' })
  class B {}
  return B;
}
",
    );
    assert_eq!(constants(&code), ["_c0"], "{code}");
    assert!(offset(&code, "import * as i0") < offset(&code, "const _c0"), "{code}");
    assert!(offset(&code, "const _c0") < offset(&code, "export const Made"), "{code}");
    assert_eq!(squash(&code).matches("ngContentSelectors:_c0").count(), 2, "{code}");
}

#[test]
fn compiles_a_nested_class_referencing_its_own_or_local_classes() {
    let code = transform(
        r"
import { Component, forwardRef, InjectionToken } from '@angular/core';
const T = new InjectionToken('t');
export function f() {
  @Component({ selector: 'h', template: '', standalone: true })
  class Helper {}
  @Component({
    selector: 'sf',
    template: '<h></h>',
    standalone: true,
    imports: [Helper],
    providers: [{ provide: T, useExisting: forwardRef(() => Sf) }, { provide: 'x', useFactory: (a: any) => a, deps: [T] }],
  })
  class Sf {}
  return Sf;
}
",
    );
    assert!(!code.contains("@Component("), "{code}");
    let squashed = squash(&code);
    assert!(squashed.contains("useExisting:forwardRef(()=>Sf)"), "{code}");
    assert!(squashed.contains("i0.ɵɵgetComponentDepsFactory(Sf,[Helper])"), "{code}");
}

#[test]
fn compiles_a_nested_class_in_partial_mode() {
    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    let code = transform_with(IIFE, &options).code;
    let squashed = squash(&code);
    assert!(!code.contains("@Component("), "{code}");
    assert!(squashed.contains("classInner{staticɵfac=i0.ɵɵngDeclareFactory("), "{code}");
    assert!(squashed.contains("staticɵcmp=i0.ɵɵngDeclareComponent({"), "{code}");
    assert!(offset(&code, "ɵɵngDeclareClassMetadata(") < offset(&code, "return Inner;"), "{code}");
}

#[test]
fn keeps_the_default_import_a_nested_factory_injects() {
    // ngtsc: `i0.ɵɵdirectiveInject(Svc)` names the default import as written.
    let code = transform(
        r"
import { Component } from '@angular/core';
import Svc from './svc';
export function mk() {
  @Component({ selector: 'ct', template: '', standalone: true })
  class Ct { constructor(s: Svc) {} }
  return Ct;
}
",
    );
    assert!(code.contains("import Svc from './svc';"), "{code}");
    assert!(code.contains("i0.ɵɵdirectiveInject(Svc)"), "{code}");
}

#[test]
fn leaves_a_nested_class_out_of_the_dts() {
    let result = transform_with(IIFE, &TransformOptions::default());
    let classes: Vec<_> = result.dts_declarations.iter().map(|d| d.class_name.as_str()).collect();
    assert_eq!(classes, ["Outer"]);
}

#[test]
fn reads_and_updates_the_template_of_a_nested_class() {
    let source = r"
import { Component } from '@angular/core';
export function f() {
  @Component({ selector: 'n', templateUrl: './n.html', standalone: true })
  class N {}
  return N;
}
";
    let allocator = Allocator::default();
    let urls = extract_component_resource_urls(&allocator, "/app/n.ts", source, None);
    assert_eq!(urls.template_urls, ["./n.html"]);

    let resources = ResolvedResources {
        templates: std::iter::once(("./n.html".to_owned(), "<i>new</i>".to_owned())).collect(),
        styles: std::collections::HashMap::default(),
    };
    let input = ComponentHmrUpdateInput { class_name: "N", ..Default::default() };
    let update = compile_component_hmr_update(
        &allocator,
        "/app/n.ts",
        source,
        &input,
        None,
        Some(&resources),
    );
    assert!(update.diagnostics.is_empty(), "{:?}", update.diagnostics);
    assert!(update.module.is_some());
    assert!(update.template_js.contains("\"i\""), "{}", update.template_js);
}

#[test]
fn compiles_two_nested_classes_of_one_name_without_hmr() {
    // HMR finds a component by `{path}@{name}`, which names neither `Tp`: an
    // update for one would replace the other's template too.
    let source = r"
import { Component } from '@angular/core';
export function a() {
  @Component({ selector: 'a', template: '<i>a</i>', standalone: true })
  class Tp {}
  return Tp;
}
export function b() {
  @Component({ selector: 'b', template: '<b>b</b>', standalone: true })
  class Tp {}
  return Tp;
}
@Component({ selector: 'c', template: '<u>c</u>', standalone: true })
export class Other {}
";
    let result = transform_with(source, &TransformOptions { hmr: true, ..Default::default() });
    let code = &result.code;
    assert_eq!(code.matches("ɵɵdefineComponent({type:Tp,").count(), 2, "{code}");
    assert!(!code.contains("Tp_HmrLoad"), "{code}");
    assert!(code.contains("Other_HmrLoad"), "{code}");
    assert!(!result.template_updates.keys().any(|id| id.ends_with("@Tp")), "{code}");

    let allocator = Allocator::default();
    let input = ComponentHmrUpdateInput { class_name: "Tp", ..Default::default() };
    let update = compile_component_hmr_update(&allocator, "test.ts", source, &input, None, None);
    assert!(update.module.is_none());
}
