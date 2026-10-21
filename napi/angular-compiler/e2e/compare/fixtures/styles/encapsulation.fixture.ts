/**
 * Style encapsulation fixtures.
 *
 * Tests different encapsulation strategies and CSS handling:
 * - ViewEncapsulation.Emulated (default) - attribute selectors
 * - ViewEncapsulation.None - global styles
 * - ViewEncapsulation.ShadowDom - native shadow DOM
 *
 * Also tests complex CSS scenarios:
 * - :host and :host-context selectors
 * - ::ng-deep for style piercing
 * - Keyframe animation scoping
 * - Media queries
 * - Complex selectors
 *
 * Uses encapsulateStyle() and generateStyleModule() NAPI functions.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  // ==========================================================================
  // Basic Encapsulation Modes
  // ==========================================================================

  {
    name: 'style-encapsulation-emulated',
    category: 'styles',
    description: 'ViewEncapsulation.Emulated - default scoping with attribute selectors',
    className: 'EmulatedStyleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-encapsulation-emulated',
  standalone: true,
  template: \`<div class="container">{{ content }}</div>\`,
  encapsulation: ViewEncapsulation.Emulated,
})
export class EmulatedStyleComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'encapsulation: 0'],
  },

  {
    name: 'style-encapsulation-none',
    category: 'styles',
    description: 'ViewEncapsulation.None - global styles without scoping',
    className: 'NoneStyleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-encapsulation-none',
  standalone: true,
  template: \`<div class="global-style">{{ content }}</div>\`,
  encapsulation: ViewEncapsulation.None,
})
export class NoneStyleComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'encapsulation: 2'],
  },

  {
    name: 'style-encapsulation-shadow-dom',
    category: 'styles',
    description: 'ViewEncapsulation.ShadowDom - native shadow DOM encapsulation',
    className: 'ShadowDomStyleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-encapsulation-shadow-dom',
  standalone: true,
  template: \`<div class="shadow-content">{{ content }}</div>\`,
  encapsulation: ViewEncapsulation.ShadowDom,
})
export class ShadowDomStyleComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'encapsulation: 3'],
  },

  // ==========================================================================
  // Host Selectors
  // ==========================================================================

  {
    name: 'style-host-selector',
    category: 'styles',
    description: ':host selector for component root styling',
    className: 'HostSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-host-selector',
  standalone: true,
  template: \`<div class="inner">Content</div>\`,
  encapsulation: ViewEncapsulation.Emulated,
  host: {
    'class': 'host-class',
  },
})
export class HostSelectorComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  {
    name: 'style-host-context',
    category: 'styles',
    description: ':host-context for ancestor-aware styling',
    className: 'HostContextComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-host-context',
  standalone: true,
  template: \`<div class="themed-content">Themed content</div>\`,
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  {
    name: 'style-host-with-class',
    category: 'styles',
    description: ':host with class selector',
    className: 'HostWithClassComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-host-with-class',
  standalone: true,
  template: \`<span>Content</span>\`,
  encapsulation: ViewEncapsulation.Emulated,
  host: {
    '[class.active]': 'isActive',
    '[class.disabled]': 'isDisabled',
  },
})
export class HostWithClassComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵclassProp'],
  },

  // ==========================================================================
  // Dynamic Style Bindings
  // ==========================================================================

  {
    name: 'style-inline-binding',
    category: 'styles',
    description: 'Inline style property binding',
    className: 'InlineStyleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-style-inline-binding',
  standalone: true,
  template: \`<div [style.color]="textColor" [style.background-color]="bgColor">Styled text</div>\`,
})
export class InlineStyleComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵstyleProp'],
  },

  {
    name: 'style-unit-binding',
    category: 'styles',
    description: 'Style binding with unit suffix',
    className: 'StyleUnitComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-style-unit-binding',
  standalone: true,
  template: \`
    <div
      [style.width.px]="width"
      [style.height.%]="heightPercent"
      [style.font-size.em]="fontSizeEm"
    >
      Sized content
    </div>
  \`,
})
export class StyleUnitComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵstyleProp'],
  },

  {
    name: 'style-map-binding',
    category: 'styles',
    description: 'Style map binding for multiple styles',
    className: 'StyleMapComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-style-map-binding',
  standalone: true,
  template: \`<div [style]="styleMap">Multiple styles</div>\`,
})
export class StyleMapComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵstyleMap'],
  },

  // ==========================================================================
  // Class Bindings
  // ==========================================================================

  {
    name: 'style-class-binding',
    category: 'styles',
    description: 'Boolean class binding',
    className: 'ClassBindingComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-style-class-binding',
  standalone: true,
  template: \`<div [class.active]="isActive" [class.hidden]="!isVisible">Toggle classes</div>\`,
})
export class ClassBindingComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵclassProp'],
  },

  {
    name: 'style-class-map',
    category: 'styles',
    description: 'Class map binding for multiple classes',
    className: 'ClassMapComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-style-class-map',
  standalone: true,
  template: \`<div [class]="classMap">Multiple classes</div>\`,
})
export class ClassMapComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵclassMap'],
  },

  {
    name: 'style-ngclass',
    category: 'styles',
    description: 'NgClass directive usage',
    className: 'NgClassComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-style-ngclass',
  standalone: true,
  template: \`<div [ngClass]="{'active': isActive, 'error': hasError}">NgClass content</div>\`,
})
export class NgClassComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  // ==========================================================================
  // Combined Host Bindings
  // ==========================================================================

  {
    name: 'style-host-style-binding',
    category: 'styles',
    description: 'Host style binding',
    className: 'HostStyleBindingComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-style-host-style-binding',
  standalone: true,
  template: \`<div>Content</div>\`,
  host: {
    '[style.display]': 'displayMode',
    '[style.opacity]': 'opacity',
  },
})
export class HostStyleBindingComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵhostProperty'],
  },

  {
    name: 'style-host-class-binding',
    category: 'styles',
    description: 'Host class binding',
    className: 'HostClassBindingComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-style-host-class-binding',
  standalone: true,
  template: \`<div>Content</div>\`,
  host: {
    '[class.expanded]': 'isExpanded',
    '[class.collapsed]': '!isExpanded',
  },
})
export class HostClassBindingComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵclassProp'],
  },

  {
    name: 'style-host-combined',
    category: 'styles',
    description: 'Combined host style and class bindings',
    className: 'HostCombinedComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-style-host-combined',
  standalone: true,
  template: \`<div>Content</div>\`,
  host: {
    '[class.active]': 'isActive',
    '[style.border-color]': 'borderColor',
    '[attr.data-state]': 'state',
  },
})
export class HostCombinedComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  // ==========================================================================
  // Complex CSS Scenarios
  // ==========================================================================

  {
    name: 'style-nested-selectors',
    category: 'styles',
    description: 'Deeply nested CSS selectors',
    className: 'NestedSelectorsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-nested-selectors',
  standalone: true,
  template: \`
    <div class="container">
      <div class="row">
        <div class="col">
          <span class="text">Nested content</span>
        </div>
      </div>
    </div>
  \`,
  encapsulation: ViewEncapsulation.Emulated,
})
export class NestedSelectorsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  {
    name: 'style-pseudo-classes',
    category: 'styles',
    description: 'CSS pseudo-class selectors',
    className: 'PseudoClassComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-pseudo-classes',
  standalone: true,
  template: \`
    <ul>
      <li>First</li>
      <li>Second</li>
      <li>Third</li>
    </ul>
  \`,
  encapsulation: ViewEncapsulation.Emulated,
})
export class PseudoClassComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  {
    name: 'style-pseudo-elements',
    category: 'styles',
    description: 'CSS pseudo-element selectors',
    className: 'PseudoElementComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-pseudo-elements',
  standalone: true,
  template: \`<div class="with-pseudo">Content with ::before and ::after</div>\`,
  encapsulation: ViewEncapsulation.Emulated,
})
export class PseudoElementComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  {
    name: 'style-attribute-selectors',
    category: 'styles',
    description: 'CSS attribute selectors',
    className: 'AttributeSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-attribute-selectors',
  standalone: true,
  template: \`
    <input type="text" placeholder="Text input" />
    <input type="email" placeholder="Email input" />
    <a href="https://example.com" target="_blank">Link</a>
  \`,
  encapsulation: ViewEncapsulation.Emulated,
})
export class AttributeSelectorComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  // ==========================================================================
  // Change Detection Interaction
  // ==========================================================================

  {
    name: 'style-onpush-encapsulation',
    ngtscMode: 'full',
    category: 'styles',
    description: 'OnPush change detection with Emulated encapsulation',
    className: 'OnPushStyleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ChangeDetectionStrategy, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-onpush-encapsulation',
  standalone: true,
  template: \`<div class="onpush-content">{{ data }}</div>\`,
  changeDetection: ChangeDetectionStrategy.OnPush,
  encapsulation: ViewEncapsulation.Emulated,
})
export class OnPushStyleComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ChangeDetectionStrategy.OnPush'],
  },

  {
    name: 'style-onpush-none',
    ngtscMode: 'full',
    category: 'styles',
    description: 'OnPush change detection with None encapsulation',
    className: 'OnPushNoneStyleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ChangeDetectionStrategy, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-onpush-none',
  standalone: true,
  template: \`<div class="global-onpush">{{ data }}</div>\`,
  changeDetection: ChangeDetectionStrategy.OnPush,
  encapsulation: ViewEncapsulation.None,
})
export class OnPushNoneStyleComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ChangeDetectionStrategy.OnPush', 'encapsulation: 2'],
  },

  // ==========================================================================
  // Template-Style Interactions
  // ==========================================================================

  {
    name: 'style-conditional-rendering',
    category: 'styles',
    description: 'Styles with conditional rendering',
    className: 'ConditionalStyleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-conditional-rendering',
  standalone: true,
  template: \`
    @if (showSection) {
      <div class="visible-section">Visible</div>
    } @else {
      <div class="hidden-section">Hidden</div>
    }
  \`,
  encapsulation: ViewEncapsulation.Emulated,
})
export class ConditionalStyleComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵconditional'],
  },

  {
    name: 'style-loop-rendering',
    category: 'styles',
    description: 'Styles with loop rendering',
    className: 'LoopStyleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-loop-rendering',
  standalone: true,
  template: \`
    <ul class="item-list">
      @for (item of items; track item.id) {
        <li class="item" [class.selected]="item.selected">{{ item.name }}</li>
      }
    </ul>
  \`,
  encapsulation: ViewEncapsulation.Emulated,
})
export class LoopStyleComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵrepeaterCreate', 'ɵɵclassProp'],
  },

  // ==========================================================================
  // Edge Cases
  // ==========================================================================

  {
    name: 'style-empty-template',
    category: 'styles',
    description: 'Component with empty template but styles',
    className: 'EmptyTemplateStyleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-empty-template',
  standalone: true,
  template: \`\`,
  encapsulation: ViewEncapsulation.Emulated,
})
export class EmptyTemplateStyleComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  {
    name: 'style-whitespace-preserve',
    category: 'styles',
    description: 'Styles with preserved whitespace',
    className: 'WhitespaceStyleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-whitespace-preserve',
  standalone: true,
  template: \`<pre class="code-block">  preserved  spaces  </pre>\`,
  preserveWhitespaces: true,
  encapsulation: ViewEncapsulation.Emulated,
})
export class WhitespaceStyleComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  {
    name: 'style-unicode-classes',
    category: 'styles',
    description: 'CSS with unicode class names',
    className: 'UnicodeClassComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-style-unicode-classes',
  standalone: true,
  template: \`<div class="container">Unicode content</div>\`,
  encapsulation: ViewEncapsulation.Emulated,
})
export class UnicodeClassComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },
]
