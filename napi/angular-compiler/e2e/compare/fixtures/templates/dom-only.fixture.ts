/**
 * DOM-only template instructions.
 *
 * A full ngtsc build compiles the template of a standalone component without directive
 * dependencies with `ɵɵdomElementStart`, `ɵɵdomListener`, `ɵɵdomProperty` and the other DOM-only
 * instructions (`TemplateCompilationMode.DomOnly`). Oxc does the same when the file alone proves
 * it: the component is standalone, its `imports` and `deferredImports` are absent or `[]`, and
 * its own selector matches nothing in its template. These fixtures compare against a full build
 * (the runner's default for such files).
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'dom-only-bindings',
    category: 'templates',
    description: 'Elements, containers, listeners and bindings in a DOM-only template',
    className: 'DomOnlyBindingsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-dom-only-bindings',
  template: \`
    <div class="box" [title]="title" [attr.data-id]="id" [class.active]="active"
         [style.width.px]="width" [aria-label]="label" [innerHTML]="html"
         (click)="onClick($event)" (window:resize)="onResize()"></div>
    <input #field [value]="title" (input)="title = field.value" />
    <ng-container>{{ title }}</ng-container>
    <ng-template #tpl><span [id]="id">in template</span></ng-template>
    <a [href]="url" (keydown.enter)="onClick($event)">link</a>
    <section [@fade]="active" (@fade.done)="onResize()"></section>
    <p animate.enter="fade-in">animated</p>
  \`,
  host: {
    '[title]': 'title',
    '(click)': 'onClick($event)',
  },
})
export class DomOnlyBindingsComponent {
  title = 'hello';
  id = 1;
  active = true;
  width = 10;
  label = 'label';
  html = '<b>x</b>';
  url = '/home';
  onClick(_event: unknown) {}
  onResize() {}
}
    `.trim(),
    expectedFeatures: [
      'ɵɵdomElementStart',
      'ɵɵdomElement',
      'ɵɵdomElementContainerStart',
      'ɵɵdomTemplate',
      'ɵɵdomListener',
      'ɵɵdomProperty',
    ],
  },
  {
    name: 'dom-only-control-flow',
    category: 'templates',
    description: 'Control flow blocks and @let in a DOM-only template',
    className: 'DomOnlyControlFlowComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-dom-only-control-flow',
  template: \`
    @let total = items.length;
    @if (total > 0) {
      <ul>
        @for (item of items; track item.id; let i = $index) {
          <li (click)="select(item)" [class.even]="i % 2 === 0">{{ item.name }}</li>
        } @empty {
          <li>none</li>
        }
      </ul>
    } @else {
      <p>empty</p>
    }
    @switch (mode) {
      @case ('a') { <span>A</span> }
      @default { <span>other</span> }
    }
  \`,
})
export class DomOnlyControlFlowComponent {
  items = [{ id: 1, name: 'one' }];
  mode = 'a';
  select(_item: unknown) {}
}
    `.trim(),
    expectedFeatures: ['ɵɵdomElementStart', 'ɵɵdomListener', 'ɵɵconditionalCreate'],
  },
  {
    name: 'dom-only-defer',
    category: 'templates',
    description: '@defer blocks with no dependencies in a DOM-only template',
    className: 'DomOnlyDeferComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-dom-only-defer',
  template: \`
    @defer (on viewport; prefetch on idle) {
      <div (click)="load()">{{ title }}</div>
    } @placeholder {
      <span>placeholder</span>
    } @loading (minimum 100ms) {
      <span>loading</span>
    } @error {
      <span>error</span>
    }
    @defer (hydrate on interaction) {
      <p [title]="title">hydrated</p>
    }
  \`,
})
export class DomOnlyDeferComponent {
  title = 'deferred';
  load() {}
}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdomElementStart', 'ɵɵdomListener'],
  },
  {
    name: 'dom-only-i18n',
    category: 'templates',
    description: 'i18n blocks, attributes and ICUs in a DOM-only template',
    className: 'DomOnlyI18nComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-dom-only-i18n',
  template: \`
    <div i18n>Hello {{ name }} <b (click)="greet()">bold</b></div>
    <span i18n-title title="Static title" [attr.lang]="lang"></span>
    <p i18n>{count, plural, =0 {none} =1 {one <i>item</i>} other {{{ count }} items}}</p>
  \`,
})
export class DomOnlyI18nComponent {
  name = 'world';
  lang = 'en';
  count = 2;
  greet() {}
}
    `.trim(),
    expectedFeatures: ['ɵɵi18nStart', 'ɵɵdomElementStart', 'ɵɵdomListener'],
  },
  {
    name: 'dom-only-empty-imports',
    category: 'templates',
    description: 'imports: [] keeps a standalone component DOM-only',
    className: 'DomOnlyEmptyImportsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-dom-only-empty-imports',
  standalone: true,
  imports: [],
  template: '<button (click)="go()" [disabled]="busy">go</button>',
})
export class DomOnlyEmptyImportsComponent {
  busy = false;
  go() {}
}
    `.trim(),
    expectedFeatures: ['ɵɵdomElementStart', 'ɵɵdomListener', 'ɵɵdomProperty'],
  },
  {
    name: 'dom-only-host-directives',
    category: 'templates',
    description: 'hostDirectives apply to the host element, so the template stays DOM-only',
    className: 'DomOnlyHostDirectivesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, Directive } from '@angular/core';

@Directive({ selector: '[appTooltip]' })
export class TooltipDirective {}

@Component({
  selector: 'app-dom-only-host-directives',
  hostDirectives: [TooltipDirective],
  template: '<span appTooltip (mouseenter)="show()">hover</span>',
})
export class DomOnlyHostDirectivesComponent {
  show() {}
}
    `.trim(),
    expectedFeatures: ['ɵɵdomElementStart', 'ɵɵdomListener'],
  },
  {
    name: 'dom-only-not-for-self-reference',
    category: 'templates',
    description: 'A component that uses its own selector has itself as a directive dependency',
    className: 'TreeComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, Input } from '@angular/core';

@Component({
  selector: 'app-tree',
  template: \`
    <span (click)="toggle()">{{ depth }}</span>
    @if (depth < 3) {
      <app-tree [depth]="depth + 1"></app-tree>
    }
  \`,
})
export class TreeComponent {
  @Input() depth = 0;
  toggle() {}
}
    `.trim(),
    expectedFeatures: ['ɵɵelementStart', 'ɵɵlistener', 'ɵɵproperty'],
  },
  {
    name: 'dom-only-not-for-default-selector-self-reference',
    category: 'templates',
    description: 'A component without a selector matches <ng-component> in its own template',
    className: 'DefaultSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  template: \`
    <span (click)="toggle()">self</span>
    @if (nested) {
      <ng-component></ng-component>
    }
  \`,
})
export class DefaultSelectorComponent {
  nested = false;
  toggle() {}
}
    `.trim(),
    expectedFeatures: ['ɵɵelementStart', 'ɵɵlistener'],
  },
  {
    name: 'dom-only-self-selector-not-matched',
    category: 'templates',
    description: 'A selector that its own template does not match stays DOM-only',
    className: 'NotSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-not:not(.skip)',
  template: '<app-not class="skip" (click)="go()">a</app-not><div app-not>b</div>',
})
export class NotSelectorComponent {
  go() {}
}
    `.trim(),
    expectedFeatures: ['ɵɵdomElementStart', 'ɵɵdomListener'],
  },
]
