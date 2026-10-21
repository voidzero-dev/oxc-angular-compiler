/**
 * Full file transformation fixtures.
 *
 * Tests the transformAngularFile() NAPI which performs the complete transformation pipeline:
 * 1. Parse the TypeScript file using oxc_parser
 * 2. Find @Component decorated classes
 * 3. Inline resolved templates and styles
 * 4. Compile templates to Angular IR code
 * 5. Generate JavaScript output using oxc_codegen
 *
 * Unlike template-only fixtures, these test full TypeScript source transformation
 * including decorator extraction, class transformation, and multi-component files.
 *
 * NOTE: Currently the fixture runner uses template-only compilation (compileTemplateSync),
 * so expectedFeatures should match template output. When runner is updated to use
 * transformAngularFile(), features like ɵɵdefineComponent will be available.
 *
 * The sourceCode field documents the intended full file for when full-file transform
 * testing is implemented.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  // ==========================================================================
  // Basic Full File Transformation
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-basic',
    category: 'full-transform',
    description: 'Basic full file transformation with inline template',
    className: 'BasicComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-basic',
  standalone: true,
  template: \`<div>{{ message }}</div>\`,
})
export class BasicComponent {
  message = 'Hello World';
}
    `.trim(),
    template: `<div>{{ message }}</div>`,
    expectedFeatures: ['ɵɵdomElementStart', 'ɵɵtext', 'ɵɵtextInterpolate1'],
  },

  {
    type: 'component',
    name: 'full-transform-with-selector',
    category: 'full-transform',
    description: 'Component with custom selector',
    className: 'CustomSelectorComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'my-custom-selector',
  standalone: true,
  template: \`<span>Custom selector component</span>\`,
})
export class CustomSelectorComponent {}
    `.trim(),
    template: `<span>Custom selector component</span>`,
    selector: 'my-custom-selector',
    expectedFeatures: ['ɵɵdomElement'],
  },

  // ==========================================================================
  // Components with Property Decorators (simulated via metadata)
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-with-inputs',
    category: 'full-transform',
    description: 'Component with input bindings in template',
    className: 'InputComponent',
    sourceCode: `
import { Component, Input } from '@angular/core';

@Component({
  selector: 'app-input',
  standalone: true,
  template: \`<div [title]="title">{{ value }}</div>\`,
})
export class InputComponent {
  @Input() value = '';
  title = 'Default Title';
}
    `.trim(),
    template: `<div [title]="title">{{ value }}</div>`,
    expectedFeatures: ['ɵɵdomElementStart', 'ɵɵdomProperty', 'ɵɵtextInterpolate1'],
  },

  {
    type: 'component',
    name: 'full-transform-with-outputs',
    category: 'full-transform',
    description: 'Component with output event bindings',
    className: 'OutputComponent',
    sourceCode: `
import { Component, Output, EventEmitter } from '@angular/core';

@Component({
  selector: 'app-output',
  standalone: true,
  template: \`<button (click)="onClick()">Click me</button>\`,
})
export class OutputComponent {
  @Output() clicked = new EventEmitter<void>();

  onClick() {
    this.clicked.emit();
  }
}
    `.trim(),
    template: `<button (click)="onClick()">Click me</button>`,
    expectedFeatures: ['ɵɵdomElementStart', 'ɵɵdomListener'],
  },

  // ==========================================================================
  // Components with Encapsulation and Change Detection
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-encapsulation-none',
    category: 'full-transform',
    description: 'Component with ViewEncapsulation.None',
    className: 'NoEncapsulationComponent',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-no-encapsulation',
  standalone: true,
  encapsulation: ViewEncapsulation.None,
  template: \`<div class="global">Global styles</div>\`,
})
export class NoEncapsulationComponent {}
    `.trim(),
    template: `<div class="global">Global styles</div>`,
    encapsulation: 'None',
    // encapsulation only appears in full component definition, not template output
    expectedFeatures: ['ɵɵdomElement'],
  },

  {
    type: 'component',
    name: 'full-transform-encapsulation-shadow-dom',
    category: 'full-transform',
    description: 'Component with ViewEncapsulation.ShadowDom',
    className: 'ShadowDomComponent',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-dom',
  standalone: true,
  encapsulation: ViewEncapsulation.ShadowDom,
  template: \`<div class="shadow">Shadow DOM content</div>\`,
})
export class ShadowDomComponent {}
    `.trim(),
    template: `<div class="shadow">Shadow DOM content</div>`,
    encapsulation: 'ShadowDom',
    // encapsulation only appears in full component definition, not template output
    expectedFeatures: ['ɵɵdomElement'],
  },

  {
    type: 'component',
    name: 'full-transform-onpush',
    ngtscMode: 'full',
    category: 'full-transform',
    description: 'Component with OnPush change detection',
    className: 'OnPushComponent',
    sourceCode: `
import { Component, ChangeDetectionStrategy } from '@angular/core';

@Component({
  selector: 'app-onpush',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: \`<div>{{ data }}</div>\`,
})
export class OnPushComponent {
  data = 'OnPush data';
}
    `.trim(),
    template: `<div>{{ data }}</div>`,
    changeDetection: 'OnPush',
    // changeDetection only appears in full component definition, not template output
    expectedFeatures: ['ɵɵdomElementStart', 'ɵɵtextInterpolate1'],
  },

  // ==========================================================================
  // Components with Host Bindings
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-host-property',
    category: 'full-transform',
    description: 'Component with host property binding',
    className: 'HostPropertyComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-host-property',
  standalone: true,
  host: {
    '[class.active]': 'isActive',
    '[style.opacity]': 'opacity',
  },
  template: \`<div>Host bindings</div>\`,
})
export class HostPropertyComponent {
  isActive = true;
  opacity = 1;
}
    `.trim(),
    template: `<div>Host bindings</div>`,
    host: {
      '[class.active]': 'isActive',
      '[style.opacity]': 'opacity',
    },
    // Template output is simple div element
    expectedFeatures: ['ɵɵdomElement'],
  },

  {
    type: 'component',
    name: 'full-transform-host-listener',
    category: 'full-transform',
    description: 'Component with host event listener',
    className: 'HostListenerComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-host-listener',
  standalone: true,
  host: {
    '(click)': 'handleClick($event)',
    '(keydown.enter)': 'handleEnter()',
  },
  template: \`<div>Click or press Enter</div>\`,
})
export class HostListenerComponent {
  handleClick(event: MouseEvent) {}
  handleEnter() {}
}
    `.trim(),
    template: `<div>Click or press Enter</div>`,
    host: {
      '(click)': 'handleClick($event)',
      '(keydown.enter)': 'handleEnter()',
    },
    // Host listeners appear in hostBindings, not template - template is simple element
    expectedFeatures: ['ɵɵdomElement'],
  },

  {
    type: 'component',
    name: 'full-transform-host-attribute',
    category: 'full-transform',
    description: 'Component with static host attributes',
    className: 'HostAttributeComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-host-attr',
  standalone: true,
  host: {
    'role': 'button',
    'tabindex': '0',
    '[attr.aria-label]': 'label',
  },
  template: \`<span>Accessible button</span>\`,
})
export class HostAttributeComponent {
  label = 'Click me';
}
    `.trim(),
    template: `<span>Accessible button</span>`,
    host: {
      role: 'button',
      tabindex: '0',
      '[attr.aria-label]': 'label',
    },
    // Template output is simple span element
    expectedFeatures: ['ɵɵdomElement'],
  },

  // ==========================================================================
  // Components with Control Flow
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-if-else',
    category: 'full-transform',
    description: 'Component with @if/@else control flow',
    className: 'IfElseComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-if-else',
  standalone: true,
  template: \`
    @if (isLoggedIn) {
      <div>Welcome, {{ username }}</div>
    } @else {
      <div>Please log in</div>
    }
  \`,
})
export class IfElseComponent {
  isLoggedIn = false;
  username = '';
}
    `.trim(),
    template: `
    @if (isLoggedIn) {
      <div>Welcome, {{ username }}</div>
    } @else {
      <div>Please log in</div>
    }
  `,
    expectedFeatures: ['ɵɵconditional', 'ɵɵtemplate'],
  },

  {
    type: 'component',
    name: 'full-transform-for-loop',
    category: 'full-transform',
    description: 'Component with @for control flow',
    className: 'ForLoopComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-for-loop',
  standalone: true,
  template: \`
    @for (item of items; track item.id) {
      <div>{{ item.name }}</div>
    } @empty {
      <div>No items found</div>
    }
  \`,
})
export class ForLoopComponent {
  items: { id: number; name: string }[] = [];
}
    `.trim(),
    template: `
    @for (item of items; track item.id) {
      <div>{{ item.name }}</div>
    } @empty {
      <div>No items found</div>
    }
  `,
    expectedFeatures: ['ɵɵrepeaterCreate', 'ɵɵrepeater'],
  },

  {
    type: 'component',
    name: 'full-transform-switch',
    category: 'full-transform',
    description: 'Component with @switch control flow',
    className: 'SwitchComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-switch',
  standalone: true,
  template: \`
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
  \`,
})
export class SwitchComponent {
  status: 'loading' | 'success' | 'error' | 'idle' = 'idle';
}
    `.trim(),
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
    expectedFeatures: ['ɵɵconditional', 'ɵɵtemplate'],
  },

  // ==========================================================================
  // Components with @defer
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-defer-basic',
    category: 'full-transform',
    description: 'Component with @defer block',
    className: 'DeferBasicComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer',
  standalone: true,
  template: \`
    @defer {
      <heavy-component />
    } @loading {
      <div>Loading...</div>
    } @placeholder {
      <div>Placeholder</div>
    } @error {
      <div>Error loading component</div>
    }
  \`,
})
export class DeferBasicComponent {}
    `.trim(),
    template: `
    @defer {
      <heavy-component />
    } @loading {
      <div>Loading...</div>
    } @placeholder {
      <div>Placeholder</div>
    } @error {
      <div>Error loading component</div>
    }
  `,
    expectedFeatures: ['ɵɵdefer', 'ɵɵtemplate'],
  },

  {
    type: 'component',
    name: 'full-transform-defer-on-viewport',
    category: 'full-transform',
    description: 'Component with @defer on viewport trigger',
    className: 'DeferViewportComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-viewport',
  standalone: true,
  template: \`
    @defer (on viewport) {
      <lazy-component />
    } @placeholder {
      <div>Scroll to load</div>
    }
  \`,
})
export class DeferViewportComponent {}
    `.trim(),
    template: `
    @defer (on viewport) {
      <lazy-component />
    } @placeholder {
      <div>Scroll to load</div>
    }
  `,
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferOnViewport'],
  },

  // ==========================================================================
  // Components with @let declarations
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-let-declarations',
    category: 'full-transform',
    description: 'Component with @let template variable declarations',
    className: 'LetDeclarationsComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-let',
  standalone: true,
  template: \`
    @let doubled = value * 2;
    @let tripled = value * 3;
    <div>Value: {{ value }}, Doubled: {{ doubled }}, Tripled: {{ tripled }}</div>
  \`,
})
export class LetDeclarationsComponent {
  value = 10;
}
    `.trim(),
    template: `
    @let doubled = value * 2;
    @let tripled = value * 3;
    <div>Value: {{ value }}, Doubled: {{ doubled }}, Tripled: {{ tripled }}</div>
  `,
    expectedFeatures: ['ɵɵdeclareLet', 'ɵɵdomElementStart'],
  },

  // ==========================================================================
  // Components with Pipes
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-with-pipes',
    category: 'full-transform',
    description: 'Component using pipes in template',
    className: 'PipesComponent',
    sourceCode: `
import { Component } from '@angular/core';
import { UpperCasePipe, DatePipe, SlicePipe } from '@angular/common';

@Component({
  selector: 'app-pipes',
  standalone: true,
  imports: [UpperCasePipe, DatePipe, SlicePipe],
  template: \`
    <div>{{ name | uppercase }}</div>
    <div>{{ date | date:'short' }}</div>
    <div>{{ text | slice:0:10 }}</div>
  \`,
})
export class PipesComponent {
  name = 'hello';
  date = new Date();
  text = 'This is a long text that needs slicing';
}
    `.trim(),
    template: `
    <div>{{ name | uppercase }}</div>
    <div>{{ date | date:'short' }}</div>
    <div>{{ text | slice:0:10 }}</div>
  `,
    expectedFeatures: ['ɵɵpipe', 'ɵɵpipeBind1', 'ɵɵpipeBind2'],
  },

  // ==========================================================================
  // Components with Content Projection
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-content-projection',
    category: 'full-transform',
    description: 'Component with ng-content projection',
    className: 'ContentProjectionComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-card',
  standalone: true,
  template: \`
    <div class="card">
      <header>
        <ng-content select="[card-header]"></ng-content>
      </header>
      <main>
        <ng-content></ng-content>
      </main>
      <footer>
        <ng-content select="[card-footer]"></ng-content>
      </footer>
    </div>
  \`,
})
export class ContentProjectionComponent {}
    `.trim(),
    template: `
    <div class="card">
      <header>
        <ng-content select="[card-header]"></ng-content>
      </header>
      <main>
        <ng-content></ng-content>
      </main>
      <footer>
        <ng-content select="[card-footer]"></ng-content>
      </footer>
    </div>
  `,
    expectedFeatures: ['ɵɵprojectionDef', 'ɵɵprojection'],
  },

  // ==========================================================================
  // Components with Template References
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-template-ref',
    category: 'full-transform',
    description: 'Component with ng-template and template references',
    className: 'TemplateRefComponent',
    sourceCode: `
import { Component } from '@angular/core';
import { NgTemplateOutlet } from '@angular/common';

@Component({
  selector: 'app-template-ref',
  standalone: true,
  imports: [NgTemplateOutlet],
  template: \`
    <ng-template #greetingTemplate let-name="name">
      <div>Hello, {{ name }}!</div>
    </ng-template>
    <ng-container *ngTemplateOutlet="greetingTemplate; context: { name: 'World' }"></ng-container>
  \`,
})
export class TemplateRefComponent {}
    `.trim(),
    template: `
    <ng-template #greetingTemplate let-name="name">
      <div>Hello, {{ name }}!</div>
    </ng-template>
    <ng-container *ngTemplateOutlet="greetingTemplate; context: { name: 'World' }"></ng-container>
  `,
    expectedFeatures: ['ɵɵtemplate'],
  },

  // ==========================================================================
  // Components with Two-Way Binding
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-two-way-binding',
    category: 'full-transform',
    description: 'Component with two-way binding syntax',
    className: 'TwoWayBindingComponent',
    sourceCode: `
import { Component } from '@angular/core';
import { FormsModule } from '@angular/forms';

@Component({
  selector: 'app-two-way',
  standalone: true,
  imports: [FormsModule],
  template: \`
    <input [(ngModel)]="name" />
    <input [(value)]="value" />
    <div>Name: {{ name }}, Value: {{ value }}</div>
  \`,
})
export class TwoWayBindingComponent {
  name = '';
  value = '';
}
    `.trim(),
    template: `
    <input [(ngModel)]="name" />
    <input [(value)]="value" />
    <div>Name: {{ name }}, Value: {{ value }}</div>
  `,
    expectedFeatures: ['ɵɵtwoWayProperty', 'ɵɵtwoWayListener'],
  },

  // ==========================================================================
  // Components with preserveWhitespaces
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-preserve-whitespace',
    category: 'full-transform',
    description: 'Component with preserveWhitespaces enabled',
    className: 'PreserveWhitespaceComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-preserve-ws',
  standalone: true,
  preserveWhitespaces: true,
  template: \`<pre>  preserved   whitespace  </pre>\`,
})
export class PreserveWhitespaceComponent {}
    `.trim(),
    template: `<pre>  preserved   whitespace  </pre>`,
    preserveWhitespaces: true,
    expectedFeatures: ['ɵɵdomElementStart', 'ɵɵtext'],
  },

  // ==========================================================================
  // Complex Real-World Component
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-complex',
    ngtscMode: 'full',
    category: 'full-transform',
    description: 'Complex component with multiple features',
    className: 'ComplexComponent',
    sourceCode: `
import { Component, ChangeDetectionStrategy, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-complex',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  encapsulation: ViewEncapsulation.Emulated,
  host: {
    '[class.active]': 'isActive',
    '(click)': 'handleClick()',
  },
  template: \`
    @if (isLoading) {
      <div class="loader">Loading...</div>
    } @else {
      @for (item of items; track item.id) {
        <div [class.selected]="item.id === selectedId" (click)="select(item.id)">
          {{ item.name | uppercase }}
        </div>
      }
    }
  \`,
})
export class ComplexComponent {
  isLoading = false;
  isActive = true;
  items: { id: number; name: string }[] = [];
  selectedId: number | null = null;

  handleClick() {}
  select(id: number) { this.selectedId = id; }
}
    `.trim(),
    template: `
    @if (isLoading) {
      <div class="loader">Loading...</div>
    } @else {
      @for (item of items; track item.id) {
        <div [class.selected]="item.id === selectedId" (click)="select(item.id)">
          {{ item.name | uppercase }}
        </div>
      }
    }
  `,
    changeDetection: 'OnPush',
    encapsulation: 'Emulated',
    host: {
      '[class.active]': 'isActive',
      '(click)': 'handleClick()',
    },
    // Template features only (changeDetection/encapsulation are in component def)
    expectedFeatures: ['ɵɵconditional', 'ɵɵrepeaterCreate', 'ɵɵpipe', 'ɵɵlistener'],
  },

  // ==========================================================================
  // Non-Standalone Component (Legacy Support)
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-non-standalone',
    category: 'full-transform',
    description: 'Non-standalone component (Angular v18 style)',
    className: 'NonStandaloneComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-non-standalone',
  standalone: false,
  template: \`<div>Non-standalone component</div>\`,
})
export class NonStandaloneComponent {}
    `.trim(),
    template: `<div>Non-standalone component</div>`,
    standalone: false,
    // Template output is simple element
    expectedFeatures: ['ɵɵelement'],
  },

  // ==========================================================================
  // Component with Inline Styles
  // ==========================================================================

  {
    type: 'component',
    name: 'full-transform-with-styles',
    category: 'full-transform',
    description: 'Component with inline styles array',
    className: 'StyledComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-styled',
  standalone: true,
  styles: [
    \`.container { padding: 1rem; }\`,
    \`.title { font-size: 2rem; font-weight: bold; }\`,
  ],
  template: \`
    <div class="container">
      <h1 class="title">{{ title }}</h1>
    </div>
  \`,
})
export class StyledComponent {
  title = 'Styled Component';
}
    `.trim(),
    template: `
    <div class="container">
      <h1 class="title">{{ title }}</h1>
    </div>
  `,
    styles: [`.container { padding: 1rem; }`, `.title { font-size: 2rem; font-weight: bold; }`],
    // styles: array only appears in component definition, not template output
    expectedFeatures: ['ɵɵdomElementStart', 'ɵɵtextInterpolate1'],
  },
]
