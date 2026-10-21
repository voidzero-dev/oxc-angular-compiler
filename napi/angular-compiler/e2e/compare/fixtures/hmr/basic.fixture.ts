/**
 * HMR (Hot Module Replacement) fixtures.
 *
 * Tests HMR template compilation which generates:
 * - Template function for hot-reloading
 * - Declaration and constant extraction for @for loops
 * - Style updates for component styles
 *
 * HMR uses compileForHmrSync() and generateHmrModule() NAPI functions.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  // ==========================================================================
  // Basic HMR Template Updates
  // ==========================================================================

  {
    name: 'hmr-basic-interpolation',
    category: 'hmr',
    description: 'Basic HMR with simple interpolation',
    className: 'HmrBasicComponent',
    template: `<div>{{ message }}</div>`,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵtext', 'ɵɵtextInterpolate'],
  },

  {
    name: 'hmr-property-binding',
    category: 'hmr',
    description: 'HMR with property binding',
    className: 'HmrPropertyComponent',
    template: `<div [title]="title" [hidden]="isHidden">Content</div>`,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵdomProperty'],
  },

  {
    name: 'hmr-event-binding',
    category: 'hmr',
    description: 'HMR with event binding',
    className: 'HmrEventComponent',
    template: `<button (click)="onClick($event)">Click me</button>`,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵdomListener'],
  },

  {
    name: 'hmr-two-way-binding',
    category: 'hmr',
    description: 'HMR with two-way binding',
    className: 'HmrTwoWayComponent',
    template: `<input [(ngModel)]="value" />`,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵtwoWayProperty', 'ɵɵtwoWayListener'],
  },

  // ==========================================================================
  // HMR with Control Flow (declarations/consts extraction)
  // ==========================================================================

  {
    name: 'hmr-for-basic',
    category: 'hmr',
    description: 'HMR with @for control flow requiring declarations',
    className: 'HmrForBasicComponent',
    template: `
      @for (item of items; track item) {
        <div>{{ item }}</div>
      }
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵrepeaterCreate', 'ɵɵrepeater'],
  },

  {
    name: 'hmr-for-track-property',
    category: 'hmr',
    description: 'HMR with @for tracking by property',
    className: 'HmrForTrackComponent',
    template: `
      @for (user of users; track user.id) {
        <div>{{ user.name }} ({{ user.email }})</div>
      }
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵrepeaterCreate'],
  },

  {
    name: 'hmr-for-with-index',
    category: 'hmr',
    description: 'HMR with @for using context variables',
    className: 'HmrForIndexComponent',
    template: `
      @for (item of items; track $index; let i = $index, first = $first, last = $last) {
        <div [class.first]="first" [class.last]="last">{{ i }}: {{ item }}</div>
      }
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵrepeaterCreate'],
  },

  {
    name: 'hmr-for-nested',
    category: 'hmr',
    description: 'HMR with nested @for loops',
    className: 'HmrForNestedComponent',
    template: `
      @for (category of categories; track category.id) {
        <h2>{{ category.name }}</h2>
        @for (item of category.items; track item.id) {
          <div>{{ item.name }}</div>
        }
      }
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵrepeaterCreate'],
  },

  // ==========================================================================
  // HMR with Conditionals
  // ==========================================================================

  {
    name: 'hmr-if-basic',
    category: 'hmr',
    description: 'HMR with @if conditional',
    className: 'HmrIfBasicComponent',
    template: `
      @if (isVisible) {
        <div>Visible content</div>
      }
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵconditional'],
  },

  {
    name: 'hmr-if-else',
    category: 'hmr',
    description: 'HMR with @if/@else',
    className: 'HmrIfElseComponent',
    template: `
      @if (isLoggedIn) {
        <div>Welcome, {{ username }}</div>
      } @else {
        <div>Please log in</div>
      }
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵconditional'],
  },

  {
    name: 'hmr-switch',
    category: 'hmr',
    description: 'HMR with @switch control flow',
    className: 'HmrSwitchComponent',
    template: `
      @switch (status) {
        @case ('loading') {
          <div>Loading...</div>
        }
        @case ('success') {
          <div>Success!</div>
        }
        @case ('error') {
          <div>Error occurred</div>
        }
        @default {
          <div>Unknown status</div>
        }
      }
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵconditional'],
  },

  // ==========================================================================
  // HMR with Defer
  // ==========================================================================

  {
    name: 'hmr-defer-basic',
    category: 'hmr',
    description: 'HMR with @defer block',
    className: 'HmrDeferBasicComponent',
    template: `
      @defer {
        <heavy-component />
      } @loading {
        <div>Loading...</div>
      } @placeholder {
        <div>Placeholder</div>
      }
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵdefer'],
  },

  {
    name: 'hmr-defer-with-trigger',
    category: 'hmr',
    description: 'HMR with @defer and on viewport trigger',
    className: 'HmrDeferTriggerComponent',
    template: `
      @defer (on viewport) {
        <lazy-component />
      } @placeholder {
        <div>Loading...</div>
      }
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵdefer', 'ɵɵdeferOnViewport'],
  },

  // ==========================================================================
  // HMR with Styles
  // ==========================================================================

  {
    name: 'hmr-with-inline-styles',
    category: 'hmr',
    description: 'HMR with component that has inline styles',
    className: 'HmrWithStylesComponent',
    template: `<div class="container">{{ content }}</div>`,
    encapsulation: 'Emulated',
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  {
    name: 'hmr-style-binding',
    category: 'hmr',
    description: 'HMR with style bindings',
    className: 'HmrStyleBindingComponent',
    template: `<div [style.color]="color" [style.font-size.px]="fontSize">Styled text</div>`,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵstyleProp'],
  },

  {
    name: 'hmr-class-binding',
    category: 'hmr',
    description: 'HMR with class bindings',
    className: 'HmrClassBindingComponent',
    template: `<div [class.active]="isActive" [class.disabled]="isDisabled">Toggle classes</div>`,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵclassProp'],
  },

  // ==========================================================================
  // HMR with Complex Templates
  // ==========================================================================

  {
    name: 'hmr-template-ref',
    category: 'hmr',
    description: 'HMR with template references',
    className: 'HmrTemplateRefComponent',
    template: `
      <ng-template #myTemplate let-name="name">
        <div>Hello, {{ name }}</div>
      </ng-template>
      <ng-container *ngTemplateOutlet="myTemplate; context: { name: 'World' }"></ng-container>
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵtemplate'],
  },

  {
    name: 'hmr-projection',
    category: 'hmr',
    description: 'HMR with content projection',
    className: 'HmrProjectionComponent',
    template: `
      <div class="wrapper">
        <ng-content select="[header]"></ng-content>
        <ng-content></ng-content>
        <ng-content select="[footer]"></ng-content>
      </div>
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵprojectionDef', 'ɵɵprojection'],
  },

  {
    name: 'hmr-let-declaration',
    category: 'hmr',
    description: 'HMR with @let declarations',
    className: 'HmrLetComponent',
    template: `
      @let doubled = value * 2;
      @let tripled = value * 3;
      <div>Original: {{ value }}, Doubled: {{ doubled }}, Tripled: {{ tripled }}</div>
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵdeclareLet'],
  },

  // ==========================================================================
  // HMR Edge Cases
  // ==========================================================================

  {
    name: 'hmr-pipe-usage',
    category: 'hmr',
    description: 'HMR with pipe in template',
    className: 'HmrPipeComponent',
    template: `<div>{{ value | uppercase | slice:0:10 }}</div>`,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵpipe', 'ɵɵpipeBind1'],
  },

  {
    name: 'hmr-safe-navigation',
    category: 'hmr',
    description: 'HMR with safe navigation operator',
    className: 'HmrSafeNavComponent',
    template: `<div>{{ user?.address?.city }}</div>`,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵtextInterpolate'],
  },

  {
    name: 'hmr-nullish-coalescing',
    category: 'hmr',
    description: 'HMR with nullish coalescing',
    className: 'HmrNullishComponent',
    template: `<div>{{ value ?? 'default' }}</div>`,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵtextInterpolate'],
  },

  {
    name: 'hmr-complex-expressions',
    category: 'hmr',
    description: 'HMR with complex template expressions',
    className: 'HmrComplexComponent',
    template: `
      <div>
        {{ (condition ? value1 : value2) | uppercase }}
        <span>{{ items.length > 0 ? items[0].name : 'No items' }}</span>
      </div>
    `,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵpipe'],
  },
]
