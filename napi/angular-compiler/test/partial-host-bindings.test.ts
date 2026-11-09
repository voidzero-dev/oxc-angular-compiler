/**
 * Host bindings of partially compiled libraries (#680), once linked.
 *
 * A partial declaration keys `host.listeners` / `host.properties` by the
 * event / property name without the `(…)` / `[…]` of its `host:` key, as
 * ngtsc's `parseHostBindings` does, and the linker parses that name like a
 * full compilation: key modifiers, `window:` targets, `attr.` / `class.` /
 * `style.` bindings with units, and animations. Each case is linked with
 * Angular's linker and with Oxc's.
 */
import { existsSync, readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, relative, resolve } from 'node:path'

import { describe, expect, it } from 'vitest'

import { linkAngularPackageSync, transformAngularFileSync } from '../index.js'

/** Angular's linker, as the Angular CLI runs it on libraries. */
async function angularLink(code: string, filename: string): Promise<string> {
  const require = createRequire(import.meta.url)
  const babel = createRequire(require.resolve('@angular/compiler-cli/package.json'))('@babel/core')
  const { createEs2015LinkerPlugin } = await import('@angular/compiler-cli/linker/babel')
  const plugin = createEs2015LinkerPlugin({
    linkerJitMode: false,
    fileSystem: {
      resolve,
      exists: existsSync,
      dirname,
      relative,
      readFile: (file: string) => readFileSync(file, 'utf8'),
    } as any,
    logger: { level: 1, debug() {}, info() {}, warn() {}, error() {} },
  })
  return babel.transformSync(code, {
    filename,
    babelrc: false,
    configFile: false,
    plugins: [plugin],
  }).code
}

const LINKERS = {
  angular: angularLink,
  oxc: async (code: string, filename: string) => linkAngularPackageSync(code, filename).code,
}

/** Partially compiles `source` and links it with `linker`. */
async function link(source: string, linker: keyof typeof LINKERS): Promise<string> {
  const result = transformAngularFileSync(source, '/lib/dir.mts', { compilationMode: 'partial' })
  expect(result.errors).toEqual([])
  return LINKERS[linker](result.code, '/lib/dir.mjs')
}

describe.each(Object.keys(LINKERS) as (keyof typeof LINKERS)[])('linked by %s', (linker) => {
  it('listens to host events by name, with key modifiers and global targets', async () => {
    const code = await link(
      `
        import { Directive } from '@angular/core';
        @Directive({
          selector: '[d]',
          host: {
            '(click)': 'onClick($event)',
            '(keydown.shift.tab)': 'onShiftTab()',
            '(window:resize)': 'onResize()',
            '(document:keydown.escape)': 'onEscape()',
          },
        })
        export class D {}
      `,
      linker,
    )
    expect(code).toMatch(/ɵɵlistener\("click",\s*function D_click_HostBindingHandler/)
    expect(code).toMatch(/\("keydown\.shift\.tab",\s*function D_keydown_shift_tab_/)
    // A handler body has no `}`, so `[^}]*` stays within one listener.
    expect(code).toMatch(/\("resize",\s*function D_resize_[^}]*\},\s*i0\.ɵɵresolveWindow\)/)
    expect(code).toMatch(
      /\("keydown\.escape",\s*function D_keydown_escape_[^}]*\},\s*i0\.ɵɵresolveDocument\)/,
    )
  })

  it('binds host properties, attributes, classes and styles with units', async () => {
    const code = await link(
      `
        import { Directive } from '@angular/core';
        @Directive({
          selector: '[d]',
          host: {
            '[id]': 'id',
            '[attr.aria-label]': 'label',
            '[class.active]': 'active',
            '[style.height.px]': 'height',
          },
        })
        export class D {}
      `,
      linker,
    )
    expect(code).toMatch(/ɵɵdomProperty\("id",\s*ctx\.id\)/)
    expect(code).toMatch(/ɵɵattribute\("aria-label",\s*ctx\.label\)/)
    expect(code).toMatch(/ɵɵclassProp\("active",\s*ctx\.active\)/)
    expect(code).toMatch(/ɵɵstyleProp\("height",\s*ctx\.height,\s*"px"\)/)
  })

  it('binds legacy animation triggers and their events', async () => {
    const code = await link(
      `
        import { Directive } from '@angular/core';
        @Directive({
          selector: '[d]',
          host: { '[@fade]': 'state', '(@fade.done)': 'onDone($event)' },
        })
        export class D {}
      `,
      linker,
    )
    expect(code).toMatch(/ɵɵsyntheticHostProperty\("@fade",\s*ctx\.state\)/)
    expect(code).toMatch(/ɵɵsyntheticHostListener\("@fade\.done",/)
  })

  it('strips only the outer brackets of a host key', async () => {
    // ngtsc keeps `[y]` / `(z)` as the name, and the linker binds it as is.
    const code = await link(
      `
        import { Directive } from '@angular/core';
        @Directive({ selector: '[d]', host: { '[[y]]': 'y', '((z))': 'z()' } })
        export class D {}
      `,
      linker,
    )
    expect(code).toMatch(/ɵɵdomProperty\("\[y\]",\s*ctx\.y\)/)
    expect(code).toMatch(/ɵɵlistener\("\(z\)",\s*function D__z__HostBindingHandler/)
  })

  it('binds @HostBinding and @HostListener members of a component', async () => {
    const code = await link(
      `
        import { Component, HostBinding, HostListener } from '@angular/core';
        @Component({ selector: 'c', template: '', host: { '(keydown.enter)': 'onEnter()' } })
        export class C {
          @HostBinding('style.width.%') width = 50;
          @HostListener('keyup.control.a', ['$event']) onSelectAll(e) {}
        }
      `,
      linker,
    )
    expect(code).toMatch(/ɵɵlistener\("keydown\.enter",\s*function C_keydown_enter_/)
    expect(code).toMatch(/\("keyup\.control\.a",\s*function C_keyup_control_a_/)
    expect(code).toMatch(/ɵɵstyleProp\("width",\s*ctx\.width,\s*"%"\)/)
  })
})
