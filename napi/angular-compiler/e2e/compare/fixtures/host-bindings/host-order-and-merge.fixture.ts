/**
 * The order of host attributes, properties and listeners, and how `host:` entries merge with
 * `@HostBinding` / `@HostListener` members.
 *
 * ngtsc keeps each kind in a plain object (`parseHostBindings`), so entries stay in the order they
 * are written, except that integer-like keys come first in ascending order. A member decorator
 * assigns into the same object (`extractHostBindings`), so it replaces a `host:` entry with the same
 * key where that entry was, and a key is only bound once.
 */
import type { Fixture } from '../types.js'

function fixture(
  name: string,
  description: string,
  className: string,
  sourceCode: string,
): Fixture {
  return {
    name,
    category: 'host-bindings',
    description,
    className,
    type: 'full-transform',
    sourceCode: sourceCode.trim(),
    expectedFeatures: [],
  }
}

export const fixtures: Fixture[] = [
  fixture(
    'host-attrs-written-order',
    'Static host attributes keep the order they are written in',
    'HostAttrsOrderComponent',
    `
import { Component } from '@angular/core';

@Component({
  selector: 'app-host-attrs-order',
  template: '',
  host: {
    role: 'button',
    tabindex: '0',
    'aria-hidden': 'true',
    'data-kind': 'primary',
    id: 'main',
  },
})
export class HostAttrsOrderComponent {}
`,
  ),
  fixture(
    'host-attrs-integer-keys',
    'Integer-like host keys come first, in ascending order, like any JS object key',
    'HostAttrsIntegerKeysDirective',
    `
import { Directive } from '@angular/core';

@Directive({
  selector: '[appIntegerKeys]',
  host: {
    z: 'last',
    '10': 'ten',
    '2': 'two',
    a: 'first',
    '[20]': 'twenty',
    '[title]': 'title',
    '[3]': 'three',
  },
})
export class HostAttrsIntegerKeysDirective {
  twenty = 1;
  title = '';
  three = 3;
}
`,
  ),
  fixture(
    'host-attrs-with-class-and-style',
    'Static class and style are added after the other host attributes',
    'HostAttrsClassStyleComponent',
    `
import { Component } from '@angular/core';

@Component({
  selector: 'app-host-attrs-class-style',
  template: '',
  host: {
    class: 'a b',
    role: 'region',
    style: 'color: red',
    id: 'panel',
  },
})
export class HostAttrsClassStyleComponent {}
`,
  ),
  fixture(
    'host-attrs-empty-class',
    'An empty static class or style is not a host attribute',
    'HostAttrsEmptyClassDirective',
    `
import { Directive } from '@angular/core';

@Directive({
  selector: '[appEmptyClass]',
  host: { class: '', style: '', role: 'note' },
})
export class HostAttrsEmptyClassDirective {}
`,
  ),
  fixture(
    'host-only-empty-class',
    'A host with only an empty static class compiles no host bindings',
    'HostOnlyEmptyClassComponent',
    `
import { Component } from '@angular/core';

@Component({
  selector: 'app-only-empty-class',
  template: '',
  host: { class: '' },
})
export class HostOnlyEmptyClassComponent {}
`,
  ),
  fixture(
    'host-listener-merged-with-host-component',
    'A @HostListener replaces a host: listener on the same event, where it was written',
    'HostListenerMergeComponent',
    `
import { Component, HostListener } from '@angular/core';

@Component({
  selector: 'app-host-listener-merge',
  template: '',
  host: { '(click)': 'fromHost()', '(focus)': 'onFocus()' },
})
export class HostListenerMergeComponent {
  fromHost() {}
  onFocus() {}
  @HostListener('keydown', ['$event']) onKey(e: any) {}
  @HostListener('click') fromMember() {}
}
`,
  ),
  fixture(
    'host-listener-merged-with-host-directive',
    'A @HostListener replaces a host: listener on the same event (directive)',
    'HostListenerMergeDirective',
    `
import { Directive, HostListener } from '@angular/core';

@Directive({
  selector: '[appHostListenerMerge]',
  host: { '(click)': 'fromHost()', '(window:resize)': 'onResize()' },
})
export class HostListenerMergeDirective {
  fromHost() {}
  onResize() {}
  @HostListener('click', ['$event']) fromMember(e: any) {}
  @HostListener('mouseenter') onEnter() {}
}
`,
  ),
  fixture(
    'host-binding-merged-with-host-component',
    'A @HostBinding replaces a host: property on the same key, where it was written',
    'HostBindingMergeComponent',
    `
import { Component, HostBinding } from '@angular/core';

@Component({
  selector: 'app-host-binding-merge',
  template: '',
  host: { '[title]': 'hostTitle', '[id]': 'hostId', '[class.on]': 'flag' },
})
export class HostBindingMergeComponent {
  hostTitle = 'a';
  hostId = 'b';
  flag = true;
  @HostBinding('attr.role') role = 'button';
  @HostBinding('title') memberTitle = 'c';
}
`,
  ),
  fixture(
    'host-binding-merged-with-host-directive',
    'A @HostBinding replaces a host: property on the same key (directive)',
    'HostBindingMergeDirective',
    `
import { Directive, HostBinding } from '@angular/core';

@Directive({
  selector: '[appHostBindingMerge]',
  host: { '[title]': 'hostTitle', '[class.on]': 'flag', role: 'note' },
})
export class HostBindingMergeDirective {
  hostTitle = 'a';
  flag = true;
  @HostBinding('class.on') memberFlag = false;
  @HostBinding('tabIndex') tabIndex = 0;
}
`,
  ),
  fixture(
    'host-members-same-key',
    'Two members bound to the same host key: the later one wins',
    'HostMembersSameKeyDirective',
    `
import { Directive, HostBinding, HostListener } from '@angular/core';

@Directive({ selector: '[appSameKey]' })
export class HostMembersSameKeyDirective {
  @HostBinding('title') first = 'a';
  @HostBinding('id') id = 'x';
  @HostBinding('title') second = 'b';
  @HostListener('click') one() {}
  @HostListener('focus') focus() {}
  @HostListener('click') two() {}
}
`,
  ),
]
