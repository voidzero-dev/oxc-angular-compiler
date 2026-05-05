import { describe, it, expect } from 'vitest'

import {
  transformAngularFile,
  extractComponentUrls,
  compileTemplate,
  extractAngularComponentByAst,
} from '../index.js'

describe('transformAngularFile', () => {
  it('should transform a simple component with inline template', async () => {
    const source = `
      import { Component } from '@angular/core';

      @Component({
        selector: 'app-root',
        template: '<h1>Hello World</h1>',
      })
      export class AppComponent {}
    `

    const result = await transformAngularFile(source, 'app.component.ts', {
      hmr: true,
    })

    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('AppComponent')
    // templateUpdates is a Record (object), not a Map
    expect(Object.keys(result.templateUpdates).length).toBeGreaterThan(0)
  })

  it('should handle components without templates', async () => {
    const source = `
      import { Component } from '@angular/core';

      @Component({
        selector: 'app-root',
      })
      export class AppComponent {}
    `

    const result = await transformAngularFile(source, 'app.component.ts', {})

    // Should complete without errors
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('AppComponent')
  })

  it('should handle external template URLs with warning', async () => {
    const source = `
      import { Component } from '@angular/core';

      @Component({
        selector: 'app-root',
        templateUrl: './app.component.html',
      })
      export class AppComponent {}
    `

    // Without resolved resources, should warn about missing template
    const result = await transformAngularFile(source, 'app.component.ts', {})

    // Should have at least one warning/error about missing template
    expect(result.errors.length).toBeGreaterThan(0)
  })

  it('should compile with resolved external resources', async () => {
    const source = `
      import { Component } from '@angular/core';

      @Component({
        selector: 'app-root',
        templateUrl: './app.component.html',
      })
      export class AppComponent {}
    `

    // Pass resources as plain object (NAPI converts Map to object)
    const resolvedResources = {
      templates: { './app.component.html': '<h1>External Template</h1>' },
      styles: {},
    }

    const result = await transformAngularFile(
      source,
      'app.component.ts',
      { hmr: true },
      resolvedResources as any,
    )

    expect(result.errors).toHaveLength(0)
    expect(Object.keys(result.templateUpdates).length).toBeGreaterThan(0)
  })

  it('should minify final component styles when enabled', async () => {
    const source = `
      import { Component } from '@angular/core';

      @Component({
        selector: 'app-root',
        template: '<div class="container">Hello</div>',
        styles: ['.container { color: red; background: transparent; }'],
      })
      export class AppComponent {}
    `

    const result = await transformAngularFile(source, 'app.component.ts', {
      minifyComponentStyles: true,
    })

    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('.container[_ngcontent-%COMP%]{color:red;background:0 0}')
  })
})

describe('extractComponentUrlsSync', () => {
  it('should extract template and style URLs', async () => {
    const source = `
      import { Component } from '@angular/core';

      @Component({
        selector: 'app-root',
        templateUrl: './app.component.html',
        styleUrls: ['./app.component.css'],
      })
      export class AppComponent {}
    `

    const urls = await extractComponentUrls(source, 'app.component.ts')

    expect(urls.templateUrls).toContain('./app.component.html')
    expect(urls.styleUrls).toContain('./app.component.css')
  })
})

describe('compileTemplateSync', () => {
  it('should compile a simple template', async () => {
    const result = await compileTemplate('<h1>Hello {{ name }}</h1>', 'TestComponent', 'test.ts')

    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('function')
    expect(result.code).toContain('TestComponent_Template')
  })

  it('should compile template literal with pipe inside interpolation (CX-40791)', async () => {
    // TemplateLiteral was not handled in convert_ast_to_ir and fell through to
    // store_and_ref_expr, so the inner BindingPipe was never registered with
    // pipe_creation and the @let variable was resolved against ctx instead of the
    // local scope.
    const result = await compileTemplate(
      '@let num = 0.75; {{ `${num | percent}` }}',
      'TestComponent',
      'test.ts',
    )

    expect(result.errors).toHaveLength(0)
    // Pipe must be registered in the create block
    expect(result.code).toContain('ɵɵpipe')
    // pipeBind1 must be called in the update block
    expect(result.code).toContain('ɵɵpipeBind1')
    // @let variable must be stored
    expect(result.code).toContain('0.75')
  })

  it('should compile template literal with surrounding text and pipe (CX-40791)', async () => {
    const result = await compileTemplate(
      '@let num = 0.75; {{ `Value: ${num | percent} done` }}',
      'TestComponent',
      'test.ts',
    )

    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('ɵɵpipe')
    expect(result.code).toContain('ɵɵpipeBind1')
  })
})

describe('extractAngularComponentByAst', () => {
  it('should extract component definitions from compiled JavaScript', () => {
    // Simulating compiled Angular output
    const compiledJs = `
const _c0 = [3, "name"];
const _c1 = ["class", "container"];

function MyComponent_Template(rf, ctx) {
  if (rf & 1) {
    i0.ɵɵelementStart(0, "div");
    i0.ɵɵtext(1);
    i0.ɵɵelementEnd();
  }
  if (rf & 2) {
    i0.ɵɵadvance();
    i0.ɵɵtextInterpolate(ctx.name);
  }
}

MyComponent.ɵfac = function MyComponent_Factory(__ngFactoryType__) {
  return new (__ngFactoryType__ || MyComponent)();
};

MyComponent.ɵcmp = i0.ɵɵdefineComponent({
  type: MyComponent,
  selectors: [["my-component"]],
  standalone: true,
  template: MyComponent_Template
});
`

    const result = extractAngularComponentByAst(compiledJs, 'MyComponent')

    // Should find const declarations
    expect(result.consts.length).toBe(2)
    expect(result.consts[0]).toContain('_c0')
    expect(result.consts[1]).toContain('_c1')

    // Should find template function
    expect(result.templateFunctions.length).toBe(1)
    expect(result.templateFunctions[0]).toContain('MyComponent_Template')

    // Should find component definition
    expect(result.componentDef).toBeDefined()
    expect(result.componentDef).toContain('ɵɵdefineComponent')

    // Should find factory definition
    expect(result.factoryDef).toBeDefined()
    expect(result.factoryDef).toContain('MyComponent_Factory')
  })

  it('should handle class with static properties', () => {
    const compiledJs = `
export class MyComponent {
  static ɵfac = function MyComponent_Factory(__ngFactoryType__) {
    return new (__ngFactoryType__ || MyComponent)();
  };
  static ɵcmp = i0.ɵɵdefineComponent({
    type: MyComponent,
    template: function MyComponent_Template(rf, ctx) {}
  });
}
`

    const result = extractAngularComponentByAst(compiledJs, 'MyComponent')

    // Should find component definition from static property
    expect(result.componentDef).toBeDefined()
    expect(result.componentDef).toContain('ɵɵdefineComponent')

    // Should find factory definition from static property
    expect(result.factoryDef).toBeDefined()
    expect(result.factoryDef).toContain('MyComponent_Factory')
  })

  it('should return empty results for non-existent class', () => {
    const compiledJs = `
const _c0 = [1, 2, 3];
OtherComponent.ɵcmp = i0.ɵɵdefineComponent({});
`

    const result = extractAngularComponentByAst(compiledJs, 'NonExistent')

    expect(result.consts.length).toBe(1) // _c0 should still be found
    expect(result.templateFunctions.length).toBe(0)
    expect(result.componentDef).toBeUndefined()
    expect(result.factoryDef).toBeUndefined()
  })

  it('should handle nested template functions', () => {
    const compiledJs = `
function MyComponent_Template(rf, ctx) {}
function MyComponent_ng_template_0_Template(rf, ctx) {}
function MyComponent_Conditional_1_Template(rf, ctx) {}
function OtherComponent_Template(rf, ctx) {}
`

    const result = extractAngularComponentByAst(compiledJs, 'MyComponent')

    // Should find all MyComponent template functions
    expect(result.templateFunctions.length).toBe(3)
    expect(result.templateFunctions.some((f) => f.includes('MyComponent_Template'))).toBe(true)
    expect(
      result.templateFunctions.some((f) => f.includes('MyComponent_ng_template_0_Template')),
    ).toBe(true)
    expect(
      result.templateFunctions.some((f) => f.includes('MyComponent_Conditional_1_Template')),
    ).toBe(true)

    // Should NOT include OtherComponent's template
    expect(result.templateFunctions.some((f) => f.includes('OtherComponent'))).toBe(false)
  })
})

describe('animation host listeners', () => {
  it('should emit syntheticHostListener for @HostListener animation events', async () => {
    const source = `
      import { Component, HostListener } from '@angular/core';
      @Component({ selector: 'app-test', template: '' })
      export class TestComponent {
        @HostListener('@animation.done', ['$event'])
        onDone(event: any) {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('ɵɵsyntheticHostListener')
    expect(result.code).toContain('"@animation.done"')
    expect(result.code).not.toMatch(/ɵɵlistener\(["']@animation/)
  })

  it('should emit syntheticHostListener for @HostListener animation start phase', async () => {
    const source = `
      import { Component, HostListener } from '@angular/core';
      @Component({ selector: 'app-test', template: '' })
      export class TestComponent {
        @HostListener('@animation.start', ['$event'])
        onStart(event: any) {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('ɵɵsyntheticHostListener')
    expect(result.code).toContain('"@animation.start"')
    expect(result.code).not.toMatch(/ɵɵlistener\(["']@animation/)
  })

  it('should emit syntheticHostListener for host object animation event binding', async () => {
    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-test',
        template: '',
        host: { '(@animation.done)': 'onDone()' },
      })
      export class TestComponent {
        onDone() {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('ɵɵsyntheticHostListener')
    expect(result.code).toContain('"@animation.done"')
  })

  it('should emit correct handler function name for animation listener', async () => {
    const source = `
      import { Component, HostListener } from '@angular/core';
      @Component({ selector: 'app-test', template: '' })
      export class TestComponent {
        @HostListener('@animation.done')
        onDone() {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    // Handler name must follow Angular's pattern: ComponentName_animation@trigger.phase_HostBindingHandler
    // After sanitize: TestComponent_animation_animation_done_HostBindingHandler
    expect(result.code).toContain('TestComponent_animation_animation_done_HostBindingHandler')
  })

  it('should emit syntheticHostListener before listener when both are present (ordering)', async () => {
    // Declare the regular listener first so that without the ordering fix the
    // output order would be wrong (listener before syntheticHostListener).
    const source = `
      import { Component, HostListener } from '@angular/core';
      @Component({ selector: 'app-test', template: '' })
      export class TestComponent {
        @HostListener('click')
        onClick() {}
        @HostListener('@animation.done')
        onDone() {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('ɵɵsyntheticHostListener')
    expect(result.code).toContain('ɵɵlistener')
    // syntheticHostListener must appear before the regular listener in the output
    const syntheticIdx = result.code.indexOf('ɵɵsyntheticHostListener')
    const listenerIdx = result.code.indexOf('ɵɵlistener')
    expect(syntheticIdx).toBeLessThan(listenerIdx)
  })

  it('should emit syntheticHostListener for @Directive animation host listener', async () => {
    const source = `
      import { Directive, HostListener } from '@angular/core';
      @Directive({ selector: '[appTest]' })
      export class TestDirective {
        @HostListener('@animation.done')
        onDone() {}
      }
    `
    const result = await transformAngularFile(source, 'test.directive.ts', {})
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('ɵɵsyntheticHostListener')
    expect(result.code).toContain('"@animation.done"')
    expect(result.code).not.toMatch(/ɵɵlistener\(["']@animation/)
  })

  // Mirrors Angular compliance test: chain_synthetic_listeners.ts
  // Angular output:
  //   ɵɵsyntheticHostListener("@animation.done", fn MyComponent_animation_animation_done_HostBindingHandler)
  //                           ("@animation.start", fn MyComponent_animation_animation_start_HostBindingHandler)
  it('should match Angular compliance chain_synthetic_listeners: chained syntheticHostListener with exact handler names', async () => {
    const source = `
      import { Component, HostListener } from '@angular/core';
      @Component({
        selector: 'my-comp',
        template: '',
        host: { '(@animation.done)': 'done()' },
      })
      export class MyComponent {
        @HostListener('@animation.start')
        start() {}
        done() {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    // Both phased listeners must use syntheticHostListener — never ɵɵlistener for @animation
    expect(result.code).not.toMatch(/ɵɵlistener\(["']@animation/)
    // Exact chained structure: ɵɵsyntheticHostListener("@animation.done", fn)("@animation.start", fn)
    // (whitespace-insensitive match to allow for formatting differences)
    expect(result.code).toMatch(
      /ɵɵsyntheticHostListener\("@animation\.done",function MyComponent_animation_animation_done_HostBindingHandler[\s\S]*?\)\s*\(\s*"@animation\.start",function MyComponent_animation_animation_start_HostBindingHandler/,
    )
  })

  // Mirrors Angular compliance test: chain_synthetic_listeners_mixed.ts
  // Angular output:
  //   ɵɵsyntheticHostListener("@animation.done", fn_done)("@animation.start", fn_start);
  //   ɵɵlistener("mousedown", fn)("mouseup", fn)("click", fn);
  it('should match Angular compliance chain_synthetic_listeners_mixed: synthetic chain before regular chain', async () => {
    const source = `
      import { Component, HostListener } from '@angular/core';
      @Component({
        selector: 'my-comp',
        template: '',
        host: {
          '(mousedown)': 'mousedown()',
          '(@animation.done)': 'done()',
          '(mouseup)': 'mouseup()',
        },
      })
      export class MyComponent {
        @HostListener('@animation.start')
        start() {}
        @HostListener('click')
        click() {}
        mousedown() {}
        done() {}
        mouseup() {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    // Synthetic chain: both animation handlers chained on one syntheticHostListener call
    expect(result.code).toMatch(
      /ɵɵsyntheticHostListener\("@animation\.done",function MyComponent_animation_animation_done_HostBindingHandler[\s\S]*?\)\s*\(\s*"@animation\.start",function MyComponent_animation_animation_start_HostBindingHandler/,
    )
    // Regular chain: all three regular handlers chained on one ɵɵlistener call
    expect(result.code).toMatch(
      /ɵɵlistener\("mousedown",function MyComponent_mousedown_HostBindingHandler/,
    )
    expect(result.code).toMatch(/MyComponent_mouseup_HostBindingHandler/)
    expect(result.code).toMatch(/MyComponent_click_HostBindingHandler/)
    // Synthetic chain must come before the regular chain
    const syntheticIdx = result.code.indexOf('ɵɵsyntheticHostListener')
    const listenerIdx = result.code.indexOf('ɵɵlistener')
    expect(syntheticIdx).toBeLessThan(listenerIdx)
    // No animation events via regular ɵɵlistener
    expect(result.code).not.toMatch(/ɵɵlistener\(["']@animation/)
  })

  // Mirrors Angular's parseLegacyAnimationEventName: phase is lowercased via `.toLowerCase()`.
  // Source `@HostListener('@anim.START')` should compile identically to `@HostListener('@anim.start')`.
  it('should lowercase phase to match Angular parser', async () => {
    const source = `
      import { Component, HostListener } from '@angular/core';
      @Component({ selector: 'app-test', template: '' })
      export class TestComponent {
        @HostListener('@anim.START')
        onStart() {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('ɵɵsyntheticHostListener("@anim.start"')
    expect(result.code).toContain('TestComponent_animation_anim_start_HostBindingHandler')
  })

  // Mirrors Angular's splitAtPeriod which trims both sides of the split.
  it('should trim phase whitespace to match Angular parser', async () => {
    const source = `
      import { Component, HostListener } from '@angular/core';
      @Component({ selector: 'app-test', template: '' })
      export class TestComponent {
        @HostListener('@anim. start ')
        onStart() {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('ɵɵsyntheticHostListener("@anim.start"')
  })

  // Mirrors Angular's `_parseLegacyAnimationEvent`: a phase that is not "start" or "done"
  // still produces a ParsedEvent with that phase (an error is also reported, but code is
  // still emitted). The listener is therefore `ɵɵsyntheticHostListener("@anim.foo", fn)`,
  // not a fallback `ɵɵlistener`.
  it('should preserve bogus phase in syntheticHostListener output', async () => {
    const source = `
      import { Component, HostListener } from '@angular/core';
      @Component({ selector: 'app-test', template: '' })
      export class TestComponent {
        @HostListener('@anim.foo')
        onFoo() {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.code).toContain('ɵɵsyntheticHostListener("@anim.foo"')
    expect(result.code).toContain('TestComponent_animation_anim_foo_HostBindingHandler')
    expect(result.code).not.toContain('ɵɵlistener("anim"')
  })

  // Mirrors Angular's `sanitizeIdentifier` (parse_util.ts) which uses `/\W/g` —
  // an ASCII-only character class. Non-ASCII characters in trigger names must be
  // replaced with `_`, matching the JavaScript regex behavior.
  it('should ASCII-sanitize non-ASCII trigger characters in handler names', async () => {
    const source = `
      import { Component, HostListener } from '@angular/core';
      @Component({ selector: 'app-test', template: '' })
      export class TestComponent {
        @HostListener('@μAnim.start')
        onStart() {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    // The literal in syntheticHostListener preserves the trigger as-is (Angular
    // does not sanitize it — the @ + trigger + . + phase string is the runtime key)
    expect(result.code).toContain('ɵɵsyntheticHostListener("@μAnim.start"')
    // The handler function name MUST sanitize μ → _ (JS \W matches non-ASCII)
    expect(result.code).toContain('TestComponent_animation__Anim_start_HostBindingHandler')
    expect(result.code).not.toContain('TestComponent_animation_μAnim_start')
  })

  // Mirrors Angular's binding_parser.ts + ingest.ts behavior for `@HostListener('@trigger')`
  // without an explicit phase. Angular's parseLegacyAnimationEventName strips the `@`,
  // leaves phase=null, and createListenerOp sets isLegacyAnimationListener=false (since
  // phase is null). Reify then emits plain `ɵɵlistener(name, handler)` — no
  // syntheticHostListener, no trailing useCapture argument.
  //
  // Angular also reports an error here, but matching the host-metadata convention in
  // this codebase (binding_parser parse errors are silently dropped for host bindings
  // — see component/transform.rs and directive/compiler.rs which never inspect
  // `parse_result.errors`), we don't surface a diagnostic. Only the code output is
  // checked for parity.
  it('should emit plain ɵɵlistener for @HostListener without phase, matching Angular', async () => {
    const source = `
      import { Component, HostListener } from '@angular/core';
      @Component({ selector: 'app-test', template: '' })
      export class TestComponent {
        @HostListener('@anim')
        onAnim() {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    expect(result.code).not.toContain('ɵɵsyntheticHostListener')
    expect(result.code).toMatch(
      /ɵɵlistener\(\s*"anim"\s*,\s*function TestComponent_anim_HostBindingHandler\(\)\s*\{[\s\S]*?\}\s*\)\s*;/,
    )
    expect(result.code).not.toMatch(/ɵɵlistener\(\s*"anim"[\s\S]*?,\s*null\s*,\s*true\s*\)/)
  })
})

describe('object spread in template bindings', () => {
  it('should preserve spread syntax in object literal bindings', async () => {
    const source = `
      import { Component } from '@angular/core'
      @Component({
        template: \`<div [title]="{ ...base, extra: 'val' }"></div>\`,
      })
      export class TestComponent {
        base = {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    // Spread is inside a pure function body (Angular memoizes object literals).
    // The emitted code should contain spread syntax, not an empty-string key.
    expect(result.code).toContain('...')
    expect(result.code).not.toMatch(/""\s*:/)
  })

  it('should preserve multiple spreads in object literal bindings', async () => {
    const source = `
      import { Component } from '@angular/core'
      @Component({
        template: \`<div [title]="{ ...a, ...b, key: 'val' }"></div>\`,
      })
      export class TestComponent {
        a = {}
        b = {}
      }
    `
    const result = await transformAngularFile(source, 'test.component.ts', {})
    expect(result.errors).toHaveLength(0)
    expect(result.code).not.toMatch(/""\s*:/)
    // Both spread variables should appear in the output
    expect(result.code).toContain('ctx.a')
    expect(result.code).toContain('ctx.b')
  })
})
