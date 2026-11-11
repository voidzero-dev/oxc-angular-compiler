//! `@boundary` / `@error` blocks (Angular 22.3, issue #700).
//!
//! Every expected output below was checked against ngc 22.3.0-next.4: both
//! outputs were formatted with oxfmt and compared line by line. Our emitter
//! adds parentheses that ngc leaves out, so the expectations are written in our
//! format and compared without whitespace.
//!
//! Each case is also compiled in partial mode and linked; the linked template
//! must match the full compilation.

use oxc_allocator::Allocator;
use oxc_angular_compiler::parser::html::HtmlParser;
use oxc_angular_compiler::transform::html_to_r3::{
    HtmlToR3Transform, TransformOptions as R3Options,
};
use oxc_angular_compiler::{CompilationMode, TransformOptions, link, transform_angular_file};

fn strip(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn compile(source: &str, mode: CompilationMode) -> String {
    let allocator = Allocator::default();
    let options = TransformOptions { compilation_mode: mode, ..Default::default() };
    let result = transform_angular_file(&allocator, "app.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "unexpected errors: {:?}", result.diagnostics);
    result.code
}

/// From the first template function to the class metadata IIFE.
fn emitted_section(code: &str) -> &str {
    let start = code.find("function ").expect("no template function");
    let end = code[start..].find("(() =>").map_or(code.len(), |i| start + i);
    &code[start..end]
}

fn check_full(source: &str, expected: &str) {
    let code = compile(source, CompilationMode::Full);
    assert_eq!(strip(emitted_section(&code)), strip(expected), "full output:\n{code}");
}

/// The parts of a compiled component that come from the template: the child template
/// functions, `decls`/`vars`, the consts and the main template function.
///
/// i18n consts are left out: the linker names i18n messages differently (it has no
/// external message ids), which has nothing to do with boundaries.
fn template_parts(code: &str) -> [String; 4] {
    let code = strip(code);
    let functions_start = code.find("function").expect("no template function");
    let functions_end = code.find("exportclass").expect("no class");
    let decls = code.find("decls:").expect("no decls");
    let template = code.find("template:function").expect("no template function");
    let end = ["},dependencies", "},encapsulation"]
        .iter()
        .filter_map(|marker| code[template..].find(marker))
        .min()
        .map_or(code.len(), |i| template + i);
    let decls_vars: String = code[decls..].split(',').take(2).collect::<Vec<_>>().join(",");
    let consts = code[decls + decls_vars.len()..template].to_string();
    let consts = if consts.contains("i18n") { String::new() } else { consts };
    [
        code[functions_start..functions_end].to_string(),
        decls_vars,
        consts,
        code[template..end].to_string(),
    ]
}

fn check_linked(source: &str) {
    let full = compile(source, CompilationMode::Full);
    let partial = compile(source, CompilationMode::Partial);
    assert!(partial.contains("ɵɵngDeclareComponent"), "partial output:\n{partial}");
    let allocator = Allocator::default();
    // The partial output keeps the TypeScript of the input file.
    let linked = link(&allocator, &partial, "app.ts");
    assert!(linked.linked, "the linker did not link:\n{partial}");
    assert_eq!(template_parts(&linked.code), template_parts(&full), "linked:\n{}", linked.code);
}

/// The R3 transform errors of `template`, as `(message, start, end)`.
fn diagnostics(template: &str) -> Vec<(String, u32, u32)> {
    let allocator = Allocator::default();
    let html = HtmlParser::new(&allocator, template, "test.html").parse();
    assert!(html.errors.is_empty(), "HTML errors: {:?}", html.errors);
    let r3 =
        HtmlToR3Transform::new(&allocator, template, R3Options::default()).transform(&html.nodes);
    r3.errors.iter().map(|e| (e.msg.clone(), e.span.start.offset, e.span.end.offset)).collect()
}

fn diag(msg: &str, start: u32, end: u32) -> (String, u32, u32) {
    (msg.to_string(), start, end)
}

// ============================================================================
// Diagnostics (messages and spans checked against `parseTemplate` of
// @angular/compiler 22.3.0-next.4)
// ============================================================================

#[test]
fn boundary_with_parameters() {
    assert_eq!(
        diagnostics("@boundary (x) { a } @error { b }"),
        vec![diag("@boundary block cannot have parameters", 0, 19)]
    );
}

#[test]
fn error_block_without_boundary() {
    assert_eq!(
        diagnostics("@error { b }"),
        vec![diag("@error block can only be used after an @defer or @boundary block.", 0, 12)]
    );
}

#[test]
fn duplicate_let_variable() {
    assert_eq!(
        diagnostics("@boundary { a } @error (let e = $error, e = $reset) { b }"),
        vec![diag("Duplicate \"let\" parameter variable \"e\"", 24, 50)]
    );
    // The implicit `$error` counts as declared.
    assert_eq!(
        diagnostics("@boundary { a } @error (let $error = $reset) { b }"),
        vec![diag("Duplicate \"let\" parameter variable \"$error\"", 24, 43)]
    );
}

#[test]
fn let_name_not_an_identifier() {
    assert_eq!(
        diagnostics("@boundary { a } @error (let 1e = $error) { b }"),
        vec![diag("\"let\" parameter must be a valid JavaScript identifier", 24, 39)]
    );
}

#[test]
fn unknown_context_variable() {
    assert_eq!(
        diagnostics("@boundary { a } @error (let e = $foo) { b }"),
        vec![diag(
            "Unknown context variable \"$foo\". Only \"$error\" and \"$reset\" are allowed",
            24,
            36
        )]
    );
}

#[test]
fn invalid_let_parameter() {
    let msg = "Invalid @error block \"let\" parameter. Parameter should match the pattern \"<name> = <variable name>\"";
    assert_eq!(diagnostics("@boundary { a } @error (let ) { b }"), vec![diag(msg, 24, 28)]);
    assert_eq!(
        diagnostics("@boundary { a } @error (let e = $error,) { b }"),
        vec![diag(msg, 24, 39)]
    );
}

#[test]
fn two_when_expressions() {
    assert_eq!(
        diagnostics("@boundary { a } @error (when a; when b) { b }"),
        vec![diag("@error block can only have one \"when\" expression", 32, 38)]
    );
}

#[test]
fn unrecognized_parameter() {
    assert_eq!(
        diagnostics("@boundary { a } @error (foo) { b }"),
        vec![diag("Unrecognized @error block parameter \"foo\"", 24, 27)]
    );
}

#[test]
fn unconditional_error_blocks() {
    assert_eq!(
        diagnostics("@boundary { a } @error { b } @error { c }"),
        vec![
            diag(
                "Unconditional @error block must be the last @error block in the boundary chain",
                16,
                28
            ),
            diag("@boundary block can only have one unconditional @error block", 29, 41),
        ]
    );
    assert_eq!(
        diagnostics("@boundary { a } @error { b } @error (when x) { c }"),
        vec![diag(
            "Unconditional @error block must be the last @error block in the boundary chain",
            16,
            28
        )]
    );
}

#[test]
fn valid_parameters() {
    for template in [
        "@boundary { a }",
        "@boundary { a } @error (let e) { b }",
        "@boundary { a } @error (let $foo) { b }",
        "@boundary { a } @error (when ) { b }",
        "@boundary { a } @error (let e = $error; let f = $reset) { b }",
        "@boundary { a } @error (e = $error) { b }",
        "@boundary { a } @error (when\nx) { b }",
        "@boundary { a } @error (when x === 'a;b') { b }",
        "@boundary { a } @error (when a) { b } @error (when b) { c } @error { d }",
    ] {
        assert_eq!(diagnostics(template), vec![], "{template:?}");
    }
}

#[test]
fn invalid_when_expression() {
    // ngc reports `Parser Error: ...` at the parameter span; like `@defer (when ...)`,
    // the expression error is reported here with the message of the expression parser.
    let errors = diagnostics("@boundary { a } @error (when a +) { b }");
    assert!(
        errors.iter().any(|(msg, ..)| msg.contains("Unexpected end of expression")),
        "{errors:?}"
    );
}

// ============================================================================
// Pipes and directives used only inside the blocks
// ============================================================================

/// A pipe in a `when` condition. ngc 22.3.0-next.4 shares the condition between the
/// op's conditions and the collapsed expression, so it creates the pipe twice in one slot
/// (`ɵɵpipe(2, "whenPipe")` twice, slot 1 unused). We create it once, after the
/// boundary, as for any other control-flow condition.
#[test]
fn pipe_in_when_condition() {
    let source = r#"import { Component, Pipe } from '@angular/core';

@Pipe({ name: 'whenPipe' })
export class WhenPipe {
  transform(v: unknown) {
    return v;
  }
}

@Component({
  selector: 'app',
  imports: [WhenPipe],
  template: `@boundary { a } @error (let e; when (e | whenPipe)) { b }`,
})
export class App {}
"#;
    let code = strip(&compile(source, CompilationMode::Full));
    assert!(code.contains("decls:4,vars:5"), "{code}");
    assert!(
        code.contains(
            "i0.ɵɵboundaryCreate(0);i0.ɵɵpipe(1,\"whenPipe\");i0.ɵɵconditionalBranchCreate(2,"
        ),
        "{code}"
    );
    assert!(
        code.contains("i0.ɵɵboundaryUpdate(0,((bnd_r1.error===null)?2:(i0.ɵɵpipeBind1(1,3,bnd_r1.error)?3:-1)),2);"),
        "{code}"
    );
}

// ============================================================================
// Output parity (generated from the cases checked against ngc)
// ============================================================================

/// Compliance golden `boundary/basic`.
#[test]
fn golden_basic() {
    let source = r##"import {Component} from '@angular/core';

@Component({
  template: `
    @boundary {
      Main Content
    } @error {
      Fallback Content
    }
  `
})
export class TestComponent {}"##;
    check_full(
        source,
        r##"function TestComponent_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Main Content "); }
}
function TestComponent_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Fallback Content "); }
}


export class TestComponent {
static ɵfac = function TestComponent_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || TestComponent)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:TestComponent,selectors:[["ng-component"]],
    decls:3,vars:1,template:function TestComponent_Template(rf,ctx) {
      if ((rf & 1)) {
        i0.ɵɵboundaryCreate(0);
        i0.ɵɵconditionalBranchCreate(1,TestComponent_Primary_1_Template,1,0)(2,TestComponent_Error_2_Template,
            1,0);
      }
      if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
          1); }
    },encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// Compliance golden `boundary/error_declaration`.
#[test]
fn golden_error_declaration() {
    let source = r##"import {Component} from '@angular/core';

@Component({
  template: `
    @boundary {
      Main Content
    } @error (let err) {
      Error: {{err.message}}
    }
  `
})
export class TestComponent {}"##;
    check_full(
        source,
        r##"function TestComponent_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Main Content "); }
}
function TestComponent_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) { i0.ɵɵtextInterpolate1(" Error: ",ctx.$error.message," "); }
}


export class TestComponent {
static ɵfac = function TestComponent_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || TestComponent)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:TestComponent,selectors:[["ng-component"]],
    decls:3,vars:1,template:function TestComponent_Template(rf,ctx) {
      if ((rf & 1)) {
        i0.ɵɵboundaryCreate(0);
        i0.ɵɵconditionalBranchCreate(1,TestComponent_Primary_1_Template,1,0)(2,TestComponent_Error_2_Template,
            1,1);
      }
      if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
          1); }
    },encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// Compliance golden `boundary/multiple_error_blocks`: two reads of `bnd` keep the variable.
#[test]
fn golden_multiple_error_blocks() {
    let source = r##"import {Component} from '@angular/core';

@Component({
  template: `
    @boundary {
      Main Content
    } @error (let err; when err.message === '404') {
      Not Found
    } @error (let err) {
      Generic Error
    }
  `
})
export class TestComponent {}"##;
    check_full(
        source,
        r##"function TestComponent_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Main Content "); }
}
function TestComponent_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Not Found "); }
}
function TestComponent_Error_3_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Generic Error "); }
}


export class TestComponent {
static ɵfac = function TestComponent_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || TestComponent)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:TestComponent,selectors:[["ng-component"]],
    decls:4,vars:1,template:function TestComponent_Template(rf,ctx) {
      if ((rf & 1)) {
        i0.ɵɵboundaryCreate(0);
        i0.ɵɵconditionalBranchCreate(1,TestComponent_Primary_1_Template,1,0)(2,TestComponent_Error_2_Template,
            1,0)(3,TestComponent_Error_3_Template,1,0);
      }
      if ((rf & 2)) {
        const bnd_r1 = i0.ɵɵgetBoundary(0);
        i0.ɵɵboundaryUpdate(0,((bnd_r1.error === null)? 1: ((bnd_r1.error.message === "404")? 2: 3)),
            1);
      }
    },encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// Compliance golden `boundary/nested`.
#[test]
fn golden_nested() {
    let source = r##"import {Component} from '@angular/core';

@Component({
  template: `
    @boundary {
      Outer Main
      @boundary {
        Inner Main
      } @error {
        Inner Fallback
      }
    } @error {
      Outer Fallback
    }
  `
})
export class TestComponent {}"##;
    check_full(
        source,
        r##"function TestComponent_Primary_1_Primary_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Inner Main "); }
}
function TestComponent_Primary_1_Error_3_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Inner Fallback "); }
}
function TestComponent_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵtext(0," Outer Main ");
    i0.ɵɵboundaryCreate(1);
    i0.ɵɵconditionalBranchCreate(2,TestComponent_Primary_1_Primary_2_Template,1,0)(3,
        TestComponent_Primary_1_Error_3_Template,1,0);
  }
  if ((rf & 2)) {
    i0.ɵɵadvance();
    i0.ɵɵboundaryUpdate(1,((i0.ɵɵgetBoundary(1).error === null)? 2: 3),2);
  }
}
function TestComponent_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Outer Fallback "); }
}


export class TestComponent {
static ɵfac = function TestComponent_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || TestComponent)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:TestComponent,selectors:[["ng-component"]],
    decls:3,vars:1,template:function TestComponent_Template(rf,ctx) {
      if ((rf & 1)) {
        i0.ɵɵboundaryCreate(0);
        i0.ɵɵconditionalBranchCreate(1,TestComponent_Primary_1_Template,4,1)(2,TestComponent_Error_2_Template,
            1,0);
      }
      if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
          1); }
    },encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// Compliance golden `boundary/retry_alias`.
#[test]
fn golden_retry_alias() {
    let source = r##"import { Component } from '@angular/core';

@Component({
  template: `
    @boundary {
      Main Content
    } @error (let err, retry = $reset) {
      <button (click)="retry()">Retry</button>
    }
  `
})
export class TestComponent {}"##;
    check_full(
        source,
        r##"function TestComponent_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Main Content "); }
}
function TestComponent_Error_2_Template(rf,ctx) {
  if ((rf & 1)) {
    const _r1 = i0.ɵɵgetCurrentView();
    i0.ɵɵdomElementStart(0,"button",0);
    i0.ɵɵdomListener("click",function TestComponent_Error_2_Template_button_click_0_listener() {
      const ctx_r1 = i0.ɵɵrestoreView(_r1);
      return i0.ɵɵresetView(ctx_r1.$reset());
    });
    i0.ɵɵtext(1,"Retry");
    i0.ɵɵdomElementEnd();
  }
}


export class TestComponent {
static ɵfac = function TestComponent_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || TestComponent)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:TestComponent,selectors:[["ng-component"]],
    decls:3,vars:1,consts:[[3,"click"]],template:function TestComponent_Template(rf,
        ctx) {
      if ((rf & 1)) {
        i0.ɵɵboundaryCreate(0);
        i0.ɵɵconditionalBranchCreate(1,TestComponent_Primary_1_Template,1,0)(2,TestComponent_Error_2_Template,
            2,0);
      }
      if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
          1); }
    },encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// Compliance golden `boundary/retry_implicit`.
#[test]
fn golden_retry_implicit() {
    let source = r##"import { Component } from '@angular/core';

@Component({
  template: `
    @boundary {
      Main Content
    } @error {
      <button (click)="$reset()">Retry</button>
    }
  `
})
export class TestComponent {}"##;
    check_full(
        source,
        r##"function TestComponent_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Main Content "); }
}
function TestComponent_Error_2_Template(rf,ctx) {
  if ((rf & 1)) {
    const _r1 = i0.ɵɵgetCurrentView();
    i0.ɵɵdomElementStart(0,"button",0);
    i0.ɵɵdomListener("click",function TestComponent_Error_2_Template_button_click_0_listener() {
      const ctx_r1 = i0.ɵɵrestoreView(_r1);
      return i0.ɵɵresetView(ctx_r1.$reset());
    });
    i0.ɵɵtext(1,"Retry");
    i0.ɵɵdomElementEnd();
  }
}


export class TestComponent {
static ɵfac = function TestComponent_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || TestComponent)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:TestComponent,selectors:[["ng-component"]],
    decls:3,vars:1,consts:[[3,"click"]],template:function TestComponent_Template(rf,
        ctx) {
      if ((rf & 1)) {
        i0.ɵɵboundaryCreate(0);
        i0.ɵɵconditionalBranchCreate(1,TestComponent_Primary_1_Template,1,0)(2,TestComponent_Error_2_Template,
            2,0);
      }
      if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
          1); }
    },encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// Compliance golden `boundary/when_condition`: no unconditional block gives `-1`.
#[test]
fn golden_when_condition() {
    let source = r##"import {Component} from '@angular/core';

@Component({
  template: `
    @boundary {
      Main Content
    } @error (let err; when err.message === '404') {
      Not Found
    }
  `
})
export class TestComponent {}"##;
    check_full(
        source,
        r##"function TestComponent_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Main Content "); }
}
function TestComponent_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," Not Found "); }
}


export class TestComponent {
static ɵfac = function TestComponent_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || TestComponent)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:TestComponent,selectors:[["ng-component"]],
    decls:3,vars:1,template:function TestComponent_Template(rf,ctx) {
      if ((rf & 1)) {
        i0.ɵɵboundaryCreate(0);
        i0.ɵɵconditionalBranchCreate(1,TestComponent_Primary_1_Template,1,0)(2,TestComponent_Error_2_Template,
            1,0);
      }
      if ((rf & 2)) {
        const bnd_r1 = i0.ɵɵgetBoundary(0);
        i0.ɵɵboundaryUpdate(0,((bnd_r1.error === null)? 1: ((bnd_r1.error.message === "404")? 2: -1)),
            1);
      }
    },encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// A `@boundary` without `@error`.
#[test]
fn no_error_block() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { x {{a}} }`,
})
export class App { a = 1; }"##;
    check_full(
        source,
        r##"function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext();
    i0.ɵɵtextInterpolate1(" x ",ctx_r0.a," ");
  }
}

export class App { a = 1; 
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:2,vars:1,template:function App_Template(rf,
    ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,1,1);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: -1),
      1); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// `when` reading component fields, `$error`, and an unconditional block last.
#[test]
fn when_with_component_fields() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { a } @error (when isCritical) { crit } @error (let e; when $any(e).code === code && $error) { code } @error { generic }`,
})
export class App { isCritical = true; code = 1; }"##;
    check_full(
        source,
        r##"function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," a "); }
}
function App_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," crit "); }
}
function App_Error_3_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," code "); }
}
function App_Error_4_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," generic "); }
}

export class App { isCritical = true; code = 1; 
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:5,vars:1,template:function App_Template(rf,
    ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,1,0)(2,App_Error_2_Template,
        1,0)(3,App_Error_3_Template,1,0)(4,App_Error_4_Template,1,0);
  }
  if ((rf & 2)) {
    const bnd_r1 = i0.ɵɵgetBoundary(0);
    i0.ɵɵboundaryUpdate(0,((bnd_r1.error === null)? 1: (ctx.isCritical? 2: (((bnd_r1.error.code === ctx.code) && bnd_r1.error)? 3: 4))),
        1);
  }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// Two `when` blocks, each with its own `$error` alias.
#[test]
fn multiple_when_conditions() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { a } @error (let e; when $any(e).code === 1 || $any(e).code === 2) { one } @error (let x; when $any(x).status) { two }`,
})
export class App {  }"##;
    check_full(
        source,
        r##"function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," a "); }
}
function App_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," one "); }
}
function App_Error_3_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," two "); }
}

export class App {  
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:4,vars:1,template:function App_Template(rf,
    ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,1,0)(2,App_Error_2_Template,
        1,0)(3,App_Error_3_Template,1,0);
  }
  if ((rf & 2)) {
    const bnd_r1 = i0.ɵɵgetBoundary(0);
    i0.ɵɵboundaryUpdate(0,((bnd_r1.error === null)? 1: (((bnd_r1.error.code === 1) || (bnd_r1.error.code === 2))? 2: (bnd_r1.error.status? 3: -1))),
        1);
  }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// Only `$error` aliases are rewritten in `when`; a `$reset` alias reads the component.
#[test]
fn reset_alias_in_when() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { a } @error (let r = $reset; when r) { x } @error (let err = $error, again = $reset) { {{err}} <button (click)="again()">t</button> }`,
})
export class App { r = 1; }"##;
    check_full(
        source,
        r##"function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," a "); }
}
function App_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," x "); }
}
function App_Error_3_Template(rf,ctx) {
  if ((rf & 1)) {
    const _r1 = i0.ɵɵgetCurrentView();
    i0.ɵɵtext(0);
    i0.ɵɵdomElementStart(1,"button",0);
    i0.ɵɵdomListener("click",function App_Error_3_Template_button_click_1_listener() {
      const ctx_r1 = i0.ɵɵrestoreView(_r1);
      return i0.ɵɵresetView(ctx_r1.$reset());
    });
    i0.ɵɵtext(2,"t");
    i0.ɵɵdomElementEnd();
  }
  if ((rf & 2)) { i0.ɵɵtextInterpolate1(" ",ctx.$error," "); }
}

export class App { r = 1; 
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:4,vars:1,consts:[[3,
    "click"]],template:function App_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,1,0)(2,App_Error_2_Template,
        1,0)(3,App_Error_3_Template,3,1);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: (ctx.r? 2: 3)),
      1); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// An `@error` view reading the parent context and its alias, with a listener.
#[test]
fn error_view_reads_parent() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `<div>{{a}}</div>@boundary { {{a}} } @error (let e) { <button (click)="retry(e)">{{a}} {{e.message}} {{e.message}}</button> }`,
})
export class App { a = 1; retry(e: unknown) {} }"##;
    check_full(
        source,
        r##"function App_Primary_3_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext();
    i0.ɵɵtextInterpolate1(" ",ctx_r0.a," ");
  }
}
function App_Error_4_Template(rf,ctx) {
  if ((rf & 1)) {
    const _r2 = i0.ɵɵgetCurrentView();
    i0.ɵɵdomElementStart(0,"button",0);
    i0.ɵɵdomListener("click",function App_Error_4_Template_button_click_0_listener() {
      const ctx_r2 = i0.ɵɵrestoreView(_r2);
      const ctx_r0 = i0.ɵɵnextContext();
      return i0.ɵɵresetView(ctx_r0.retry(ctx_r2.$error));
    });
    i0.ɵɵtext(1);
    i0.ɵɵdomElementEnd();
  }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext();
    i0.ɵɵadvance();
    i0.ɵɵtextInterpolate3("",ctx_r0.a," ",ctx.$error.message," ",ctx.$error.message);
  }
}

export class App { a = 1; retry(e: unknown) {} 
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:5,vars:2,consts:[[3,
    "click"]],template:function App_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵdomElementStart(0,"div");
    i0.ɵɵtext(1);
    i0.ɵɵdomElementEnd();
    i0.ɵɵboundaryCreate(2);
    i0.ɵɵconditionalBranchCreate(3,App_Primary_3_Template,1,1)(4,App_Error_4_Template,
        2,3);
  }
  if ((rf & 2)) {
    i0.ɵɵadvance();
    i0.ɵɵtextInterpolate(ctx.a);
    i0.ɵɵadvance();
    i0.ɵɵboundaryUpdate(2,((i0.ɵɵgetBoundary(2).error === null)? 3: 4),3);
  }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// A boundary inside an `@error` block: `e` is the outer error inside the inner views.
#[test]
fn nested_in_error_block() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { a } @error (let e) { @boundary { {{e}} } @error (let e2) { {{e}} {{e2}} } }`,
})
export class App {  }"##;
    check_full(
        source,
        r##"function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," a "); }
}
function App_Error_2_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext();
    i0.ɵɵtextInterpolate1(" ",ctx_r0.$error," ");
  }
}
function App_Error_2_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext();
    i0.ɵɵtextInterpolate2(" ",ctx_r0.$error," ",ctx.$error," ");
  }
}
function App_Error_2_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Error_2_Primary_1_Template,1,1)(2,App_Error_2_Error_2_Template,
        1,2);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
      1); }
}

export class App {  
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:3,vars:1,template:function App_Template(rf,
    ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,1,0)(2,App_Error_2_Template,
        3,1);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
      1); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// Local refs and `@let` in the primary view; `$reset`/`$error` in a listener.
#[test]
fn local_ref_and_let() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { <input #inp> @let v = a + 1; {{inp.value}} {{v}} } @error { <button (click)="$reset(); log($error); a = a + 1">x</button> }`,
})
export class App { a = 1; log(e: unknown) {} }"##;
    check_full(
        source,
        r##"function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵdomElement(0,"input",null,0);
    i0.ɵɵtext(2);
  }
  if ((rf & 2)) {
    const inp_r1 = i0.ɵɵreference(1);
    const v_r2 = (i0.ɵɵnextContext().a + 1);
    i0.ɵɵadvance(2);
    i0.ɵɵtextInterpolate2(" ",inp_r1.value," ",v_r2," ");
  }
}
function App_Error_2_Template(rf,ctx) {
  if ((rf & 1)) {
    const _r3 = i0.ɵɵgetCurrentView();
    i0.ɵɵdomElementStart(0,"button",1);
    i0.ɵɵdomListener("click",function App_Error_2_Template_button_click_0_listener() {
      const ctx_r3 = i0.ɵɵrestoreView(_r3);
      const ctx_r4 = i0.ɵɵnextContext();
      ctx_r3.$reset();
      ctx_r4.log(ctx_r3.$error);
      return i0.ɵɵresetView((ctx_r4.a = (ctx_r4.a + 1)));
    });
    i0.ɵɵtext(1,"x");
    i0.ɵɵdomElementEnd();
  }
}

export class App { a = 1; log(e: unknown) {} 
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:3,vars:1,consts:[["inp",
    ""],[3,"click"]],template:function App_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,3,2)(2,App_Error_2_Template,
        2,0);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
      1); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// A boundary inside `@if`.
#[test]
fn inside_if() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@if (x) { @boundary { {{a}} } @error (let e) { {{e.message}} {{x}} } }`,
})
export class App { x = true; a = 1; }"##;
    check_full(
        source,
        r##"function App_Conditional_0_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext(2);
    i0.ɵɵtextInterpolate1(" ",ctx_r0.a," ");
  }
}
function App_Conditional_0_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext(2);
    i0.ɵɵtextInterpolate2(" ",ctx.$error.message," ",ctx_r0.x," ");
  }
}
function App_Conditional_0_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Conditional_0_Primary_1_Template,1,1)(2,App_Conditional_0_Error_2_Template,
        1,2);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
      1); }
}

export class App { x = true; a = 1; 
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:1,vars:1,template:function App_Template(rf,
    ctx) {
  if ((rf & 1)) { i0.ɵɵconditionalCreate(0,App_Conditional_0_Template,3,1); }
  if ((rf & 2)) { i0.ɵɵconditional((ctx.x? 0: -1)); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// A boundary inside `@for`, with `when` reading the loop item.
#[test]
fn inside_for() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@for (item of items; track item) { @boundary { {{item}} } @error (let e; when e.code === item) { {{item}} {{e}} {{$index}} } }`,
})
export class App { items = [1]; }"##;
    check_full(
        source,
        r##"function App_For_1_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    const item_r1 = i0.ɵɵnextContext().$implicit;
    i0.ɵɵtextInterpolate1(" ",item_r1," ");
  }
}
function App_For_1_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    const ctx_r1 = i0.ɵɵnextContext();
    const item_r1 = ctx_r1.$implicit;
    const $index_r3 = ctx_r1.$index;
    i0.ɵɵtextInterpolate3(" ",item_r1," ",ctx.$error," ",$index_r3," ");
  }
}
function App_For_1_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_For_1_Primary_1_Template,1,1)(2,App_For_1_Error_2_Template,
        1,3);
  }
  if ((rf & 2)) {
    const item_r1 = ctx.$implicit;
    const bnd_r4 = i0.ɵɵgetBoundary(0);
    i0.ɵɵboundaryUpdate(0,((bnd_r4.error === null)? 1: ((bnd_r4.error.code === item_r1)? 2: -1)),
        1);
  }
}

export class App { items = [1]; 
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:2,vars:0,template:function App_Template(rf,
    ctx) {
  if ((rf & 1)) { i0.ɵɵrepeaterCreate(0,App_For_1_Template,3,1,null,null,i0.ɵɵrepeaterTrackByIdentity); }
  if ((rf & 2)) { i0.ɵɵrepeater(ctx.items); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// A boundary inside `@defer` and a `@defer` inside a boundary.
#[test]
fn defer_inside_and_around() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@defer { @boundary { <span>{{a}}</span> } @error { e } } @boundary { @defer (on idle) { {{a}} } @placeholder { p } } @error { e2 }`,
})
export class App { a = 1; }"##;
    check_full(
        source,
        r##"function App_Defer_0_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵdomElementStart(0,"span");
    i0.ɵɵtext(1);
    i0.ɵɵdomElementEnd();
  }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext(2);
    i0.ɵɵadvance();
    i0.ɵɵtextInterpolate(ctx_r0.a);
  }
}
function App_Defer_0_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," e "); }
}
function App_Defer_0_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Defer_0_Primary_1_Template,2,1,"span")(2,App_Defer_0_Error_2_Template,
        1,0);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
      1); }
}
function App_Primary_4_Defer_0_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext(2);
    i0.ɵɵtextInterpolate1(" ",ctx_r0.a," ");
  }
}
function App_Primary_4_DeferPlaceholder_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," p "); }
}
function App_Primary_4_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵdomTemplate(0,App_Primary_4_Defer_0_Template,1,1)(1,App_Primary_4_DeferPlaceholder_1_Template,
        1,0);
    i0.ɵɵdefer(2,0,null,null,1);
    i0.ɵɵdeferOnIdle();
  }
}
function App_Error_5_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," e2 "); }
}

export class App { a = 1; 
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:6,vars:1,template:function App_Template(rf,
    ctx) {
  if ((rf & 1)) {
    i0.ɵɵdomTemplate(0,App_Defer_0_Template,3,1);
    i0.ɵɵdefer(1,0);
    i0.ɵɵdeferOnIdle();
    i0.ɵɵboundaryCreate(3);
    i0.ɵɵconditionalBranchCreate(4,App_Primary_4_Template,4,0)(5,App_Error_5_Template,
        1,0);
  }
  if ((rf & 2)) {
    i0.ɵɵadvance(3);
    i0.ɵɵboundaryUpdate(3,((i0.ɵɵgetBoundary(3).error === null)? 4: 5),4);
  }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// A single root element: the primary branch gets its tag and attributes, the error branch none.
#[test]
fn root_element_attributes() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { <div class="a" title="b" [id]="t">x</div> } @error { <span class="e">e</span> }`,
})
export class App { t = "x"; }"##;
    check_full(
        source,
        r##"function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵdomElementStart(0,"div",0);
    i0.ɵɵtext(1,"x");
    i0.ɵɵdomElementEnd();
  }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext();
    i0.ɵɵdomProperty("id",ctx_r0.t);
  }
}
function App_Error_2_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵdomElementStart(0,"span",1);
    i0.ɵɵtext(1,"e");
    i0.ɵɵdomElementEnd();
  }
}

export class App { t = "x"; 
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:3,vars:1,consts:[["title",
    "b",1,"a",3,"id"],[1,"e"]],template:function App_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,2,1,"div",0)(2,App_Error_2_Template,
        2,0);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
      1); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// A boundary inside `<svg>`.
#[test]
fn svg_namespace() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `<svg>@boundary { <circle r="1"></circle> } @error { <rect></rect> }</svg>`,
})
export class App {  }"##;
    check_full(
        source,
        r##"function App_Primary_2_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵnamespaceSVG();
    i0.ɵɵdomElement(0,"circle",0);
  }
}
function App_Error_3_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵnamespaceSVG();
    i0.ɵɵdomElement(0,"rect");
  }
}

export class App {  
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:4,vars:1,consts:[["r",
    "1"]],template:function App_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵnamespaceSVG();
    i0.ɵɵdomElementStart(0,"svg");
    i0.ɵɵboundaryCreate(1);
    i0.ɵɵconditionalBranchCreate(2,App_Primary_2_Template,1,0,"circle",0)(3,App_Error_3_Template,
        1,0);
    i0.ɵɵdomElementEnd();
  }
  if ((rf & 2)) {
    i0.ɵɵadvance();
    i0.ɵɵboundaryUpdate(1,((i0.ɵɵgetBoundary(1).error === null)? 2: 3),2);
  }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// `i18n` elements inside both blocks.
#[test]
fn i18n_inside_blocks() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { <div i18n>Hi {{a}}</div> } @error { <span i18n>Oops</span> }`,
})
export class App { a = 1; }"##;
    check_full(
        source,
        r##"function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵdomElementStart(0,"div");
    i0.ɵɵi18n(1,0);
    i0.ɵɵdomElementEnd();
  }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext();
    i0.ɵɵadvance();
    i0.ɵɵi18nExp(ctx_r0.a);
    i0.ɵɵi18nApply(1);
  }
}
function App_Error_2_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵdomElementStart(0,"span");
    i0.ɵɵi18n(1,1);
    i0.ɵɵdomElementEnd();
  }
}

export class App { a = 1; 
static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:3,vars:1,consts:() =>{
  let i18n_0;
  if (((typeof ngI18nClosureMode !== "undefined") && ngI18nClosureMode)) {
    const MSG_EXTERNAL_7178488294910539188$$APP_TS_0 = goog.getMsg("Hi {$interpolation}",
        {"interpolation":"\uFFFD0\uFFFD"});
    (i18n_0 = MSG_EXTERNAL_7178488294910539188$$APP_TS_0);
  } else {
    (i18n_0 = $localize `Hi ${"\uFFFD0\uFFFD"}:INTERPOLATION:`);
  }
  let i18n_1;
  if (((typeof ngI18nClosureMode !== "undefined") && ngI18nClosureMode)) {
    const MSG_EXTERNAL_7872557156040593774$$APP_TS_1 = goog.getMsg("Oops");
    (i18n_1 = MSG_EXTERNAL_7872557156040593774$$APP_TS_1);
  } else {
    (i18n_1 = $localize `Oops`);
  }
  return [i18n_0,i18n_1];
},template:function App_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,2,1,"div")(2,App_Error_2_Template,
        2,0);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
      1); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// `@if`/`@for` in the primary view and `@switch` in the error view.
#[test]
fn around_control_flow() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { @if (x) { <b>{{a}}</b> } @else { none } @for (i of items; track i) { {{i}} } } @error (let e) { @switch (x) { @case (1) { one {{e}} } @default { d } } }`,
})
export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}
}"##;
    check_full(
        source,
        r##"function App_Primary_1_Conditional_0_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵdomElementStart(0,"b");
    i0.ɵɵtext(1);
    i0.ɵɵdomElementEnd();
  }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext(2);
    i0.ɵɵadvance();
    i0.ɵɵtextInterpolate(ctx_r0.a);
  }
}
function App_Primary_1_Conditional_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," none "); }
}
function App_Primary_1_For_3_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    const i_r2 = ctx.$implicit;
    i0.ɵɵtextInterpolate1(" ",i_r2," ");
  }
}
function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵconditionalCreate(0,App_Primary_1_Conditional_0_Template,2,1,"b")(1,App_Primary_1_Conditional_1_Template,
        1,0);
    i0.ɵɵrepeaterCreate(2,App_Primary_1_For_3_Template,1,1,null,null,i0.ɵɵrepeaterTrackByIdentity);
  }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext();
    i0.ɵɵconditional((ctx_r0.x? 0: 1));
    i0.ɵɵadvance(2);
    i0.ɵɵrepeater(ctx_r0.items);
  }
}
function App_Error_2_Case_0_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    const ctx_r2 = i0.ɵɵnextContext();
    i0.ɵɵtextInterpolate1(" one ",ctx_r2.$error," ");
  }
}
function App_Error_2_Case_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," d "); }
}
function App_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵconditionalCreate(0,App_Error_2_Case_0_Template,1,1)(1,App_Error_2_Case_1_Template,
      1,0); }
  if ((rf & 2)) {
    let tmp_7_0;
    const ctx_r0 = i0.ɵɵnextContext();
    i0.ɵɵconditional((((tmp_7_0 = ctx_r0.x) === 1)? 0: 1));
  }
}

export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}

static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:3,vars:1,template:function App_Template(rf,
    ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,4,1)(2,App_Error_2_Template,
        2,1);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
      1); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// `<ng-template>` inside both blocks.
#[test]
fn ng_template_inside() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { <ng-template #t><i>{{a}}</i></ng-template> } @error { <ng-template>err</ng-template> }`,
})
export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}
}"##;
    check_full(
        source,
        r##"function App_Primary_1_ng_template_0_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵdomElementStart(0,"i");
    i0.ɵɵtext(1);
    i0.ɵɵdomElementEnd();
  }
  if ((rf & 2)) {
    const ctx_r0 = i0.ɵɵnextContext(2);
    i0.ɵɵadvance();
    i0.ɵɵtextInterpolate(ctx_r0.a);
  }
}
function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵdomTemplate(0,App_Primary_1_ng_template_0_Template,2,1,"ng-template",
      null,0,i0.ɵɵtemplateRefExtractor); }
}
function App_Error_2_ng_template_0_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0,"err"); }
}
function App_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵdomTemplate(0,App_Error_2_ng_template_0_Template,1,0,"ng-template"); }
}

export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}

static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:3,vars:1,consts:[["t",
    ""]],template:function App_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,2,0)(2,App_Error_2_Template,
        1,0);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
      1); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// `when` reading a parent `@let`.
#[test]
fn when_reads_let() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@let limit = a + 1; @boundary { a } @error (let e; when $any(e).code > limit) { {{limit}} }`,
})
export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}
}"##;
    check_full(
        source,
        r##"function App_Primary_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," a "); }
}
function App_Error_3_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    i0.ɵɵnextContext();
    const limit_r1 = i0.ɵɵreadContextLet(0);
    i0.ɵɵtextInterpolate1(" ",limit_r1," ");
  }
}

export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}

static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:4,vars:2,template:function App_Template(rf,
    ctx) {
  if ((rf & 1)) {
    i0.ɵɵdeclareLet(0);
    i0.ɵɵboundaryCreate(1);
    i0.ɵɵconditionalBranchCreate(2,App_Primary_2_Template,1,0)(3,App_Error_3_Template,
        1,1);
  }
  if ((rf & 2)) {
    const bnd_r2 = i0.ɵɵgetBoundary(1);
    const limit_r3 = i0.ɵɵstoreLet((ctx.a + 1));
    i0.ɵɵadvance();
    i0.ɵɵboundaryUpdate(1,((bnd_r2.error === null)? 2: ((bnd_r2.error.code > limit_r3)? 3: -1)),
        2);
  }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// Two sibling boundaries: slots and `bnd` names.
#[test]
fn sibling_boundaries() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { one } @error { e1 } <p>{{a}}</p> @boundary { two } @error (let e) { {{e}} }`,
})
export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}
}"##;
    check_full(
        source,
        r##"function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," one "); }
}
function App_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," e1 "); }
}
function App_Primary_6_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," two "); }
}
function App_Error_7_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) { i0.ɵɵtextInterpolate1(" ",ctx.$error," "); }
}

export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}

static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:8,vars:3,template:function App_Template(rf,
    ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,1,0)(2,App_Error_2_Template,
        1,0);
    i0.ɵɵdomElementStart(3,"p");
    i0.ɵɵtext(4);
    i0.ɵɵdomElementEnd();
    i0.ɵɵboundaryCreate(5);
    i0.ɵɵconditionalBranchCreate(6,App_Primary_6_Template,1,0)(7,App_Error_7_Template,
        1,1);
  }
  if ((rf & 2)) {
    i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),1);
    i0.ɵɵadvance(4);
    i0.ɵɵtextInterpolate(ctx.a);
    i0.ɵɵadvance();
    i0.ɵɵboundaryUpdate(5,((i0.ɵɵgetBoundary(5).error === null)? 6: 7),6);
  }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// A `when` read of the loop item: Angular counts it twice, so `item_r2` stays a variable.
#[test]
fn when_reads_for_item() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@for (item of items; track $index) { @boundary { x } @error (when item.bad) { {{$index}} } }`,
})
export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}
}"##;
    check_full(
        source,
        r##"function App_For_1_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," x "); }
}
function App_For_1_Error_2_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0); }
  if ((rf & 2)) {
    const $index_r1 = i0.ɵɵnextContext().$index;
    i0.ɵɵtextInterpolate1(" ",$index_r1," ");
  }
}
function App_For_1_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_For_1_Primary_1_Template,1,0)(2,App_For_1_Error_2_Template,
        1,1);
  }
  if ((rf & 2)) {
    const item_r2 = ctx.$implicit;
    i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: (item_r2.bad? 2: -1)),
        1);
  }
}

export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}

static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:2,vars:0,template:function App_Template(rf,
    ctx) {
  if ((rf & 1)) { i0.ɵɵrepeaterCreate(0,App_For_1_Template,3,1,null,null,i0.ɵɵrepeaterTrackByIndex); }
  if ((rf & 2)) { i0.ɵɵrepeater(ctx.items); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// A local ref and the error aliases used in an `@error` listener.
#[test]
fn error_ref_in_listener() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { a } @error (let e, retry = $reset) { <input #box> <button (click)="retry(); log(box.value, e)">r</button> }`,
})
export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}
}"##;
    check_full(
        source,
        r##"function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," a "); }
}
function App_Error_2_Template(rf,ctx) {
  if ((rf & 1)) {
    const _r1 = i0.ɵɵgetCurrentView();
    i0.ɵɵdomElement(0,"input",null,0);
    i0.ɵɵdomElementStart(2,"button",1);
    i0.ɵɵdomListener("click",function App_Error_2_Template_button_click_2_listener() {
      const ctx_r1 = i0.ɵɵrestoreView(_r1);
      const box_r3 = i0.ɵɵreference(1);
      const ctx_r3 = i0.ɵɵnextContext();
      ctx_r1.$reset();
      return i0.ɵɵresetView(ctx_r3.log(box_r3.value,ctx_r1.$error));
    });
    i0.ɵɵtext(3,"r");
    i0.ɵɵdomElementEnd();
  }
}

export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}

static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:3,vars:1,consts:[["box",
    ""],[3,"click"]],template:function App_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,1,0)(2,App_Error_2_Template,
        4,0);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
      1); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}

/// `@let` inside an `@error` block.
#[test]
fn let_in_error_block() {
    let source = r##"import {Component} from '@angular/core';
@Component({
  selector: 'app',
  template: `@boundary { a } @error (let e) { @let msg = $any(e).message; <span [title]="msg">{{msg}}</span> }`,
})
export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}
}"##;
    check_full(
        source,
        r##"function App_Primary_1_Template(rf,ctx) {
  if ((rf & 1)) { i0.ɵɵtext(0," a "); }
}
function App_Error_2_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵdomElementStart(0,"span",0);
    i0.ɵɵtext(1);
    i0.ɵɵdomElementEnd();
  }
  if ((rf & 2)) {
    const msg_r1 = ctx.$error.message;
    i0.ɵɵdomProperty("title",msg_r1);
    i0.ɵɵadvance();
    i0.ɵɵtextInterpolate(msg_r1);
  }
}

export class App {
  a = 1;
  x = 1;
  items = [1, 2];
  log(...args: unknown[]) {}

static ɵfac = function App_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || App)();
};
static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({type:App,selectors:[["app"]],decls:3,vars:1,consts:[[3,
    "title"]],template:function App_Template(rf,ctx) {
  if ((rf & 1)) {
    i0.ɵɵboundaryCreate(0);
    i0.ɵɵconditionalBranchCreate(1,App_Primary_1_Template,1,0)(2,App_Error_2_Template,
        2,2);
  }
  if ((rf & 2)) { i0.ɵɵboundaryUpdate(0,((i0.ɵɵgetBoundary(0).error === null)? 1: 2),
      1); }
},encapsulation:2});
}"##,
    );
    check_linked(source);
}
