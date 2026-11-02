//! The `forbidOrphanComponents` option (`angularCompilerOptions`) adds
//! `forbidOrphanRendering: true` to every component's `ɵsetClassDebugInfo`
//! call, standalone or not. ngtsc's ComponentDecoratorHandler passes the flag
//! to `extractClassDebugInfo`, and `compileClassDebugInfo` writes the field
//! only when it is set. Directives and pipes get no debug info, and partial
//! output has none at all.
//!
//! Ported from Angular's `ngtsc_spec.ts` "should set forbidOrphanRendering
//! debug info for component when the option forbidOrphanComponents is set"
//! (and the standalone and option-unset cases next to it). The expected calls
//! are what ngtsc 22.2.3 prints for the same input.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{CompilationMode, TransformOptions, transform_angular_file};

const SOURCE: &str = "import { Component, Directive, NgModule, Pipe } from '@angular/core';

@Directive({ selector: '[d]', standalone: false })
export class Dir {}

@Pipe({ name: 'p', standalone: false })
export class P { transform(v: any) { return v; } }

@Component({
  selector: 'comp',
  template: '<span d>{{ 1 | p }}</span>',
  standalone: false,
})
export class Comp {}

@NgModule({ declarations: [Comp, Dir, P] })
export class Mod {}

@Component({ selector: 'sa', template: 'sa', standalone: true })
class Sa {}

export { Sa };
";

fn compile(options: &TransformOptions) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "b.ts", SOURCE, Some(options), None);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    result.code
}

/// The `ɵsetClassDebugInfo(...)` calls, without whitespace (the emitter wraps
/// long lines), so they compare with ngtsc's printed output.
fn debug_info_calls(code: &str) -> Vec<String> {
    code.match_indices("ɵsetClassDebugInfo(")
        .map(|(start, _)| {
            let rest = &code[start..];
            let end = rest.find("})").expect("call end") + 2;
            rest[..end].chars().filter(|c| !c.is_whitespace()).collect()
        })
        .collect()
}

#[test]
fn sets_forbid_orphan_rendering_on_every_component_when_the_option_is_set() {
    // ngtsc 22.2.3, `forbidOrphanComponents: true`, full mode:
    //   i0.ɵsetClassDebugInfo(Comp, { className: "Comp", filePath: "b.ts", lineNumber: 14, forbidOrphanRendering: true })
    //   i0.ɵsetClassDebugInfo(Sa, { className: "Sa", filePath: "b.ts", lineNumber: 20, forbidOrphanRendering: true })
    let options = TransformOptions { forbid_orphan_components: true, ..Default::default() };
    let code = compile(&options);
    assert_eq!(
        debug_info_calls(&code),
        [
            r#"ɵsetClassDebugInfo(Comp,{className:"Comp",filePath:"b.ts",lineNumber:14,forbidOrphanRendering:true})"#,
            r#"ɵsetClassDebugInfo(Sa,{className:"Sa",filePath:"b.ts",lineNumber:20,forbidOrphanRendering:true})"#,
        ],
        "{code}"
    );
}

#[test]
fn leaves_debug_info_unchanged_when_the_option_is_unset() {
    // ngtsc 22.2.3, option unset, full mode: the same calls without the field.
    let code = compile(&TransformOptions::default());
    assert_eq!(
        debug_info_calls(&code),
        [
            r#"ɵsetClassDebugInfo(Comp,{className:"Comp",filePath:"b.ts",lineNumber:14})"#,
            r#"ɵsetClassDebugInfo(Sa,{className:"Sa",filePath:"b.ts",lineNumber:20})"#,
        ],
        "{code}"
    );
    assert!(!code.contains("forbidOrphanRendering"), "{code}");
}

#[test]
fn partial_mode_emits_no_debug_info_with_the_option_set() {
    // ngtsc 22.2.3, `forbidOrphanComponents: true`, partial mode: compilePartial
    // passes no debug info, so there is no `ɵsetClassDebugInfo` call.
    let options = TransformOptions {
        forbid_orphan_components: true,
        compilation_mode: CompilationMode::Partial,
        ..Default::default()
    };
    let code = compile(&options);
    assert!(!code.contains("setClassDebugInfo"), "{code}");
    assert!(!code.contains("forbidOrphanRendering"), "{code}");
}
