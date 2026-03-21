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
