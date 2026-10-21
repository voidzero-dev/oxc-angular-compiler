/**
 * Shadow CSS encapsulation fixtures.
 *
 * Tests the CSS encapsulation/shimming logic that transforms CSS selectors
 * to be scoped to the component. This covers:
 * - Basic selector scoping with attribute selectors
 * - :host and :host() pseudo-class handling
 * - :host-context() ancestor-aware styling
 * - ::ng-deep style piercing
 * - Pseudo-function selectors (:where, :is, :has, :not)
 * - Keyframe and animation scoping
 * - Complex selector combinations
 *
 * These tests verify that Oxc's Rust CSS encapsulation produces identical
 * output to Angular's TypeScript shadow_css.ts implementation.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  // ==========================================================================
  // Basic Selector Scoping
  // ==========================================================================

  {
    name: 'shadow-css-basic-selector',
    category: 'styles',
    description: 'Basic selector scoping with attribute selector',
    className: 'BasicSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-basic-selector',
  standalone: true,
  template: \`<div class="one">Content</div>\`,
  styles: [\`one {color: red;} two {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class BasicSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-multiple-selectors',
    category: 'styles',
    description: 'Multiple selectors in a rule',
    className: 'MultipleSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-multiple-selectors',
  standalone: true,
  template: \`<div class="one two">Content</div>\`,
  styles: [\`one, two {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class MultipleSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-descendant-selectors',
    category: 'styles',
    description: 'Descendant combinator selectors',
    className: 'DescendantSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-descendant-selectors',
  standalone: true,
  template: \`<div class="one"><span class="two">Content</span></div>\`,
  styles: [\`one two {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class DescendantSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-child-combinator',
    category: 'styles',
    description: 'Child combinator selector',
    className: 'ChildCombinatorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-child-combinator',
  standalone: true,
  template: \`<div class="one"><span class="two">Content</span></div>\`,
  styles: [\`one > two {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class ChildCombinatorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-sibling-combinators',
    category: 'styles',
    description: 'Adjacent and general sibling combinators',
    className: 'SiblingCombinatorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-sibling-combinators',
  standalone: true,
  template: \`<div class="one"></div><div class="two"></div>\`,
  styles: [\`one + two {color: red;} one ~ two {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class SiblingCombinatorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-attribute-selectors',
    category: 'styles',
    description: 'Various attribute selector syntaxes',
    className: 'AttributeSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-attribute-selectors',
  standalone: true,
  template: \`<input type="text" />\`,
  styles: [\`one[attr="value"] {color: red;} one[attr=value] {color: blue;} one[attr^="val"] {color: green;} one[attr$="ue"] {color: yellow;} one[attr*="alu"] {color: orange;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class AttributeSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-pseudo-elements',
    category: 'styles',
    description: 'Pseudo-element selectors',
    className: 'PseudoElementComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-pseudo-elements',
  standalone: true,
  template: \`<div class="one">Content</div>\`,
  styles: [\`one::before {content: "x";} one::after {content: "y";}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class PseudoElementComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-compound-class',
    category: 'styles',
    description: 'Compound class selectors',
    className: 'CompoundClassComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-compound-class',
  standalone: true,
  template: \`<div class="one two">Content</div>\`,
  styles: [\`.one.two {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class CompoundClassComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  // ==========================================================================
  // :host Selectors
  // ==========================================================================

  {
    name: 'shadow-css-host-basic',
    category: 'styles',
    description: ':host selector without context',
    className: 'HostBasicComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-basic',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host {display: block;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostBasicComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-with-class',
    category: 'styles',
    description: ':host with class selector',
    className: 'HostWithClassComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-with-class',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host(.active) {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostWithClassComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-with-tag',
    category: 'styles',
    description: ':host with tag selector',
    className: 'HostWithTagComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-with-tag',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host(ul) {list-style: none;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostWithTagComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-with-attribute',
    category: 'styles',
    description: ':host with attribute selector',
    className: 'HostWithAttributeComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-with-attribute',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host([disabled]) {opacity: 0.5;} :host([a="b"]) {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostWithAttributeComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-multiple-selectors',
    category: 'styles',
    description: ':host with multiple selectors',
    className: 'HostMultipleSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-multiple-selectors',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host(ul,li) {color: red;} :host(.x,.y) {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostMultipleSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-compound-class',
    category: 'styles',
    description: ':host with compound class selector',
    className: 'HostCompoundClassComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-compound-class',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host(.a.b) {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostCompoundClassComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-pseudo-selectors',
    category: 'styles',
    description: ':host with pseudo selectors',
    className: 'HostPseudoSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-pseudo-selectors',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host(:before) {content: "x";} :host:before {content: "y";} :host:nth-child(8n+1) {color: red;} :host(.class):before {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostPseudoSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-not-selector',
    category: 'styles',
    description: ':host with :not selector',
    className: 'HostNotSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-not-selector',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host(:not(p)):before {color: red;} :host:not(.foo, .bar) {color: blue;} :host(:not(:has(p))) {color: green;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostNotSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-with-descendant',
    category: 'styles',
    description: ':host with descendant selector',
    className: 'HostDescendantComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-with-descendant',
  standalone: true,
  template: \`<div class="inner">Content</div>\`,
  styles: [\`:host .inner {color: red;} :host > .inner {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostDescendantComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-attribute-next-operator',
    category: 'styles',
    description: ':host with attribute and next operator without spaces',
    className: 'HostAttrNextOpComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-attribute-next-operator',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host[foo]>div {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostAttrNextOpComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  // ==========================================================================
  // :host-context Selectors
  // ==========================================================================

  {
    name: 'shadow-css-host-context-basic',
    category: 'styles',
    description: ':host-context with basic class selector',
    className: 'HostContextBasicComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-basic',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host-context(.theme-dark) {color: white;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextBasicComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-context-tag',
    category: 'styles',
    description: ':host-context with tag selector',
    className: 'HostContextTagComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-tag',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host-context(div) {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextTagComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-context-attribute',
    category: 'styles',
    description: ':host-context with attribute selector',
    className: 'HostContextAttributeComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-attribute',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host-context([a="b"]) {color: red;} :host-context([a=b]) {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextAttributeComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-context-with-descendant',
    category: 'styles',
    description: ':host-context with descendant selector',
    className: 'HostContextDescendantComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-with-descendant',
  standalone: true,
  template: \`<div class="inner">Content</div>\`,
  styles: [\`:host-context(.x) > .y {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextDescendantComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-context-pseudo-not',
    category: 'styles',
    description: ':host-context with pseudo selectors like :not',
    className: 'HostContextPseudoNotComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-pseudo-not',
  standalone: true,
  template: \`<div class="backdrop">Content</div>\`,
  styles: [\`:host-context(backdrop:not(.borderless)) .backdrop {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextPseudoNotComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-context-double',
    category: 'styles',
    description: 'Multiple :host-context selectors (exponential permutation)',
    className: 'HostContextDoubleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-double',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host-context(.one):host-context(.two) {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextDoubleComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-context-triple',
    category: 'styles',
    description: 'Triple :host-context selectors (exponential permutation)',
    className: 'HostContextTripleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-triple',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host-context(.X):host-context(.Y):host-context(.Z) {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextTripleComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-context-no-selector',
    category: 'styles',
    description: ':host-context with no ancestor selector',
    className: 'HostContextNoSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-no-selector',
  standalone: true,
  template: \`<div class="inner">Content</div>\`,
  styles: [\`:host-context .inner {color: red;} :host-context() .inner {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextNoSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-context-comma-list',
    category: 'styles',
    description: ':host-context with comma-separated selectors',
    className: 'HostContextCommaListComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-comma-list',
  standalone: true,
  template: \`<div class="inner">Content</div>\`,
  styles: [\`:host-context(.one,.two) .inner {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextCommaListComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-context-comma-child',
    category: 'styles',
    description: ':host-context with comma-separated child selector',
    className: 'HostContextCommaChildComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-comma-child',
  standalone: true,
  template: \`<a>Link</a>\`,
  styles: [\`:host-context(.foo) a:not(.a, .b) {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextCommaChildComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-context-nested-pseudo',
    category: 'styles',
    description: ':host-context with nested pseudo selectors',
    className: 'HostContextNestedPseudoComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-nested-pseudo',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host-context(:where(.foo:not(.bar))) {color: red;} :host-context(:is(.foo:not(.bar))) {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextNestedPseudoComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  // ==========================================================================
  // :host-context and :host Combination
  // ==========================================================================

  {
    name: 'shadow-css-host-context-host-same-element',
    category: 'styles',
    description: ':host-context and :host on same element',
    className: 'HostContextHostSameComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-host-same-element',
  standalone: true,
  template: \`<div class="y">Content</div>\`,
  styles: [\`:host-context(div):host(.x) > .y {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextHostSameComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-context-host-different',
    category: 'styles',
    description: ':host-context and :host on different elements',
    className: 'HostContextHostDifferentComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-context-host-different',
  standalone: true,
  template: \`<div class="y">Content</div>\`,
  styles: [\`:host-context(div) :host(.x) > .y {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostContextHostDifferentComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-host-context',
    category: 'styles',
    description: ':host followed by :host-context',
    className: 'HostHostContextComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-host-context',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:host:host-context(.one) {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostHostContextComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  // ==========================================================================
  // ::ng-deep
  // ==========================================================================

  {
    name: 'shadow-css-ng-deep-basic',
    category: 'styles',
    description: '::ng-deep at start of selector',
    className: 'NgDeepBasicComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-ng-deep-basic',
  standalone: true,
  template: \`<div class="y">Content</div>\`,
  styles: [\`::ng-deep y {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class NgDeepBasicComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-ng-deep-after-selector',
    category: 'styles',
    description: '::ng-deep after element selector',
    className: 'NgDeepAfterSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-ng-deep-after-selector',
  standalone: true,
  template: \`<div class="y">Content</div>\`,
  styles: [\`x ::ng-deep y {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class NgDeepAfterSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-ng-deep-with-host',
    category: 'styles',
    description: '::ng-deep with :host',
    className: 'NgDeepWithHostComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-ng-deep-with-host',
  standalone: true,
  template: \`<div class="x">Content</div>\`,
  styles: [\`:host > ::ng-deep .x {color: red;} :host ::ng-deep > .x {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class NgDeepWithHostComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-deep-combinator',
    category: 'styles',
    description: '/deep/ combinator (deprecated)',
    className: 'DeepCombinatorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-deep-combinator',
  standalone: true,
  template: \`<div class="y">Content</div>\`,
  styles: [\`x /deep/ y {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class DeepCombinatorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-shadow-piercing',
    category: 'styles',
    description: '>>> combinator (deprecated)',
    className: 'ShadowPiercingComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-shadow-piercing',
  standalone: true,
  template: \`<div class="y">Content</div>\`,
  styles: [\`x >>> y {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class ShadowPiercingComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  // ==========================================================================
  // Pseudo-function Selectors (:where, :is, :has, :not)
  // ==========================================================================

  {
    name: 'shadow-css-where-basic',
    category: 'styles',
    description: ':where pseudo-function',
    className: 'WhereBasicComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-where-basic',
  standalone: true,
  template: \`<div class="one">Content</div>\`,
  styles: [\`:where(.one) {color: red;} :where(div.one span.two) {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class WhereBasicComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-where-with-host',
    category: 'styles',
    description: ':where with :host',
    className: 'WhereWithHostComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-where-with-host',
  standalone: true,
  template: \`<div class="one">Content</div>\`,
  styles: [\`:where(:host) {color: red;} :where(:host) .one {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class WhereWithHostComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-where-comma-list',
    category: 'styles',
    description: ':where with comma-separated selectors',
    className: 'WhereCommaListComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-where-comma-list',
  standalone: true,
  template: \`<div class="one two">Content</div>\`,
  styles: [\`:where(.one, .two) {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class WhereCommaListComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-where-nested',
    category: 'styles',
    description: ':where nested inside other selectors',
    className: 'WhereNestedComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-where-nested',
  standalone: true,
  template: \`<div class="one two">Content</div>\`,
  styles: [\`div :where(.one) {color: red;} :host :where(.one .two) {color: blue;} table :where(td, th):hover {color: lime;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class WhereNestedComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-is-basic',
    category: 'styles',
    description: ':is pseudo-function',
    className: 'IsBasicComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-is-basic',
  standalone: true,
  template: \`<div class="foo">Content</div>\`,
  styles: [\`div:is(.foo) {color: red;} :is(.foo) {color: blue;} :is(.foo, .bar, .baz) {color: green;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class IsBasicComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-is-with-host',
    category: 'styles',
    description: ':is with :host',
    className: 'IsWithHostComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-is-with-host',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:is(.dark :host) {color: red;} :host:is(.foo) {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class IsWithHostComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-has-basic',
    category: 'styles',
    description: ':has pseudo-function',
    className: 'HasBasicComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-has-basic',
  standalone: true,
  template: \`<div><a>Link</a></div>\`,
  styles: [\`div:has(a) {color: red;} :has(a) :has(b) {color: blue;} :has(a, b) {color: green;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HasBasicComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-has-with-host',
    category: 'styles',
    description: ':has with :host',
    className: 'HasWithHostComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-has-with-host',
  standalone: true,
  template: \`<div><a>Link</a></div>\`,
  styles: [\`div:has(a) :host {color: red;} :has(.one :host, .two) {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HasWithHostComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-not-with-comma',
    category: 'styles',
    description: ':not with comma-separated selectors',
    className: 'NotWithCommaComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-not-with-comma',
  standalone: true,
  template: \`<div class="header">Content</div>\`,
  styles: [\`.header:not(.admin) {color: red;} .header:not(.admin, :host.super .header) {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class NotWithCommaComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-nth-selectors',
    category: 'styles',
    description: ':nth-child and :nth-of-type selectors',
    className: 'NthSelectorsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-nth-selectors',
  standalone: true,
  template: \`<ul><li>Item</li></ul>\`,
  styles: [\`li:nth-last-child(-n + 3) {color: red;} dd:nth-last-of-type(3n) {color: blue;} dd:nth-of-type(even) {color: green;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class NthSelectorsComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-complex-pseudo-combo',
    category: 'styles',
    description: 'Complex combination of pseudo-functions',
    className: 'ComplexPseudoComboComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-complex-pseudo-combo',
  standalone: true,
  template: \`<div class="example-2">Content</div>\`,
  styles: [\`:host:is([foo],[foo-2])>div.example-2 {color: red;} :host:has([foo],[foo-2])>div.example-2 {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class ComplexPseudoComboComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-where-is-nested',
    category: 'styles',
    description: 'Nested :where and :is functions',
    className: 'WhereIsNestedComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-where-is-nested',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:where(:is(a)) {color: red;} :where(:is(a, b)) {color: blue;} :where(:host:is(.one, .two)) {color: green;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class WhereIsNestedComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-where-host-context',
    category: 'styles',
    description: ':where with :host-context',
    className: 'WhereHostContextComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-where-host-context',
  standalone: true,
  template: \`<div class="foo bar">Content</div>\`,
  styles: [\`:where(:host-context(backdrop)) .foo ~ .bar {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class WhereHostContextComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-double-where-host-context',
    category: 'styles',
    description: 'Double :where with :host-context (exponential)',
    className: 'DoubleWhereHostContextComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-double-where-host-context',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`:where(:host-context(.one)) :where(:host-context(.two)) {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class DoubleWhereHostContextComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  // ==========================================================================
  // Keyframes and Animations
  // ==========================================================================

  {
    name: 'shadow-css-keyframes-basic',
    category: 'styles',
    description: 'Basic keyframes scoping',
    className: 'KeyframesBasicComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-keyframes-basic',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`@keyframes foo {0% {transform: scale(0);} 100% {transform: scale(1);}}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class KeyframesBasicComponent {}
    `.trim(),
    expectedFeatures: ['styles:', '@keyframes'],
  },

  {
    name: 'shadow-css-animation-with-keyframes',
    category: 'styles',
    description: 'Animation referencing local keyframes',
    className: 'AnimationKeyframesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-animation-with-keyframes',
  standalone: true,
  template: \`<button>Click</button>\`,
  styles: [\`button {animation: foo 10s ease;} @keyframes foo {0% {transform: scale(0);} 100% {transform: scale(1);}}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class AnimationKeyframesComponent {}
    `.trim(),
    expectedFeatures: ['styles:', '@keyframes', 'animation:'],
  },

  {
    name: 'shadow-css-animation-name-scoping',
    category: 'styles',
    description: 'animation-name property scoping',
    className: 'AnimationNameComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-animation-name-scoping',
  standalone: true,
  template: \`<button>Click</button>\`,
  styles: [\`button {animation-name: foo;} @keyframes foo {0% {opacity: 0;} 100% {opacity: 1;}}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class AnimationNameComponent {}
    `.trim(),
    expectedFeatures: ['styles:', '@keyframes', 'animation-name:'],
  },

  {
    name: 'shadow-css-animation-no-local-keyframes',
    category: 'styles',
    description: 'Animation not scoped without local keyframes',
    className: 'AnimationNoLocalComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-animation-no-local-keyframes',
  standalone: true,
  template: \`<button>Click</button>\`,
  styles: [\`button {animation: foo 10s ease;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class AnimationNoLocalComponent {}
    `.trim(),
    expectedFeatures: ['styles:', 'animation:'],
  },

  {
    name: 'shadow-css-webkit-keyframes',
    category: 'styles',
    description: '-webkit-keyframes scoping',
    className: 'WebkitKeyframesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-webkit-keyframes',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`@-webkit-keyframes foo {0% {-webkit-transform: scale(0);}}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class WebkitKeyframesComponent {}
    `.trim(),
    expectedFeatures: ['styles:', '@-webkit-keyframes'],
  },

  // ==========================================================================
  // @-rules (Media, Supports, Container, etc.)
  // ==========================================================================

  {
    name: 'shadow-css-media-query',
    category: 'styles',
    description: '@media rule with scoped selectors',
    className: 'MediaQueryComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-media-query',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`@media screen and (max-width: 800px) {div {font-size: 50px;}} div {}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class MediaQueryComponent {}
    `.trim(),
    expectedFeatures: ['styles:', '@media'],
  },

  {
    name: 'shadow-css-supports',
    category: 'styles',
    description: '@supports rule with scoped selectors',
    className: 'SupportsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-supports',
  standalone: true,
  template: \`<section>Content</section>\`,
  styles: [\`@supports (display: flex) {section {display: flex;}}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class SupportsComponent {}
    `.trim(),
    expectedFeatures: ['styles:', '@supports'],
  },

  {
    name: 'shadow-css-container-query',
    category: 'styles',
    description: '@container rule with scoped selectors',
    className: 'ContainerQueryComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-container-query',
  standalone: true,
  template: \`<div class="item">Content</div>\`,
  styles: [\`@container (max-width: 500px) {.item {color: red;}}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class ContainerQueryComponent {}
    `.trim(),
    expectedFeatures: ['styles:', '@container'],
  },

  {
    name: 'shadow-css-layer',
    category: 'styles',
    description: '@layer rule with scoped selectors',
    className: 'LayerComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-layer',
  standalone: true,
  template: \`<section>Content</section>\`,
  styles: [\`@layer utilities {section {display: flex;}}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class LayerComponent {}
    `.trim(),
    expectedFeatures: ['styles:', '@layer'],
  },

  {
    name: 'shadow-css-import',
    category: 'styles',
    description: '@import directive passthrough',
    className: 'ImportComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-import',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`@import url("fonts.css"); div {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class ImportComponent {}
    `.trim(),
    expectedFeatures: ['styles:', '@import'],
  },

  {
    name: 'shadow-css-page',
    category: 'styles',
    description: '@page rule (unscoped)',
    className: 'PageComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-page',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`@page {margin-right: 4in;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class PageComponent {}
    `.trim(),
    expectedFeatures: ['styles:', '@page'],
  },

  // ==========================================================================
  // Edge Cases and Special Characters
  // ==========================================================================

  {
    name: 'shadow-css-escaped-selectors',
    category: 'styles',
    description: 'Escaped characters in selectors',
    className: 'EscapedSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-escaped-selectors',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`one\\\\/two {color: red;} one\\\\:two {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class EscapedSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-calc',
    category: 'styles',
    description: 'calc() function preserved',
    className: 'CalcComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-calc',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`div {height: calc(100% - 55px);}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class CalcComponent {}
    `.trim(),
    expectedFeatures: ['styles:', 'calc('],
  },

  {
    name: 'shadow-css-quoted-url',
    category: 'styles',
    description: 'Quoted content in url()',
    className: 'QuotedUrlComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-quoted-url',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`div {background-image: url("a.jpg"); color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class QuotedUrlComponent {}
    `.trim(),
    expectedFeatures: ['styles:', 'url('],
  },

  {
    name: 'shadow-css-escaped-quotes',
    category: 'styles',
    description: 'Escaped quotes in content',
    className: 'EscapedQuotesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-escaped-quotes',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`div::after {content: "\\\\""}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class EscapedQuotesComponent {}
    `.trim(),
    expectedFeatures: ['styles:', 'content:'],
  },

  {
    name: 'shadow-css-curly-braces-in-content',
    category: 'styles',
    description: 'Curly braces inside quoted content',
    className: 'CurlyBracesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-curly-braces-in-content',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`div::after {content: "{}"}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class CurlyBracesComponent {}
    `.trim(),
    expectedFeatures: ['styles:', 'content:'],
  },

  {
    name: 'shadow-css-closing-paren-in-url',
    category: 'styles',
    description: 'Closing parenthesis in url()',
    className: 'ClosingParenUrlComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-closing-paren-in-url',
  standalone: true,
  template: \`<p>Content</p>\`,
  styles: [\`p {background-image: url(")");} p {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class ClosingParenUrlComponent {}
    `.trim(),
    expectedFeatures: ['styles:', 'url('],
  },

  {
    name: 'shadow-css-multiline-selector',
    category: 'styles',
    description: 'Multiline selectors preserved',
    className: 'MultilineSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-multiline-selector',
  standalone: true,
  template: \`<div class="foo bar">Content</div>\`,
  styles: [\`.foo,
.bar {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class MultilineSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-comments',
    category: 'styles',
    description: 'CSS comments handling',
    className: 'CommentsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-comments',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`/* comment */ b {color: red;} /* another comment */ a {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class CommentsComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-sourcemap-comment',
    category: 'styles',
    description: 'sourceMappingURL comments preserved',
    className: 'SourcemapComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-sourcemap-comment',
  standalone: true,
  template: \`<div>Content</div>\`,
  styles: [\`b {color: red;} /*# sourceMappingURL=data:x */\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class SourcemapComponent {}
    `.trim(),
    expectedFeatures: ['styles:', 'sourceMappingURL'],
  },

  // ==========================================================================
  // Host Inclusions in Pseudo-selectors
  // ==========================================================================

  {
    name: 'shadow-css-host-in-is',
    category: 'styles',
    description: ':host inside :is selector',
    className: 'HostInIsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-in-is',
  standalone: true,
  template: \`<div class="toolbar panel">Content</div>\`,
  styles: [\`.header:is(:host > .toolbar, :host ~ .panel) {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostInIsComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-in-where',
    category: 'styles',
    description: ':host inside :where selector',
    className: 'HostInWhereComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-in-where',
  standalone: true,
  template: \`<div class="toolbar panel">Content</div>\`,
  styles: [\`.header:where(:host > .toolbar, :host ~ .panel) {color: red;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostInWhereComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-host-in-not',
    category: 'styles',
    description: ':host inside :not selector',
    className: 'HostInNotComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-host-in-not',
  standalone: true,
  template: \`<div class="header">Content</div>\`,
  styles: [\`.header:not(.admin, :host.super .header) {color: red;} .header:not(.admin, :host.super .header, :host.mega .header) {color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class HostInNotComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-mixed-host-in-where',
    category: 'styles',
    description: 'Mixed :host and regular selectors in :where',
    className: 'MixedHostWhereComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-mixed-host-in-where',
  standalone: true,
  template: \`<div class="one two">Content</div>\`,
  styles: [\`.one :where(.two, :host) {color: red;} .one :where(:host, .two) {color: blue;} :is(.foo):is(:host):is(.two) {color: green;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class MixedHostWhereComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  // ==========================================================================
  // Polyfill Directives
  // ==========================================================================

  {
    name: 'shadow-css-polyfill-next-selector',
    category: 'styles',
    description: 'polyfill-next-selector directive',
    className: 'PolyfillNextSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-polyfill-next-selector',
  standalone: true,
  template: \`<div class="x y">Content</div>\`,
  styles: [\`polyfill-next-selector {content: 'x > y'} z {}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class PolyfillNextSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-polyfill-unscoped-rule',
    category: 'styles',
    description: 'polyfill-unscoped-rule directive',
    className: 'PolyfillUnscopedComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-polyfill-unscoped-rule',
  standalone: true,
  template: \`<div id="menu" class="bar">Content</div>\`,
  styles: [\`polyfill-unscoped-rule {content: '#menu > .bar'; color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class PolyfillUnscopedComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-polyfill-rule',
    category: 'styles',
    description: 'polyfill-rule directive',
    className: 'PolyfillRuleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-polyfill-rule',
  standalone: true,
  template: \`<div class="bar">Content</div>\`,
  styles: [\`polyfill-rule {content: ':host.foo .bar'; color: blue;}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class PolyfillRuleComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  // ==========================================================================
  // Alignment with Angular v22 shadow_css.ts
  // ==========================================================================

  {
    name: 'shadow-css-trim-output',
    category: 'styles',
    description: 'Output is trimmed like ShadowCss._scopeCssText',
    className: 'TrimOutputComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-trim-output',
  standalone: true,
  template: \`<div class="a">Content</div>\`,
  styles: [\`\n\n  .a { color: red; }\n\n  \`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class TrimOutputComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-at-rule-closing-whitespace',
    category: 'styles',
    description: 'Whitespace before an at-rule closing brace is kept',
    className: 'AtRuleClosingWhitespaceComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-at-rule-closing-whitespace',
  standalone: true,
  template: \`<div class="a">Content</div>\`,
  styles: [\`@media screen { .a { color: red; } }  @supports (display: grid) {\n  .a { display: grid; }\n}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class AtRuleClosingWhitespaceComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-text-before-closing-brace',
    category: 'styles',
    description: 'Text with no block before a closing brace stays in place unscoped',
    className: 'TextBeforeClosingBraceComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-text-before-closing-brace',
  standalone: true,
  template: \`<div class="a">Content</div>\`,
  styles: [\`@media screen { .a {} b } c {}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class TextBeforeClosingBraceComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-escaped-quote-in-string',
    category: 'styles',
    description: 'An escaped quote does not end a string',
    className: 'EscapedQuoteComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-escaped-quote-in-string',
  standalone: true,
  template: \`<div class="a b">Content</div>\`,
  styles: [\`.a::before { content: "\\\\""; } .b { color: red; } .c[title="x\\\\"y"] { color: blue; }\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class EscapedQuoteComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-declaration-at-rules',
    category: 'styles',
    description:
      'Bodies of at-rules other than the scoped ones are copied unchanged (@property is left out: ngtsc 22.2 namespaces its name to --%NS%x outside ShadowCss)',
    className: 'DeclarationAtRulesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-declaration-at-rules',
  standalone: true,
  template: \`<div class="a">Content</div>\`,
  styles: [\`@counter-style thumbs { system: cyclic; symbols: "*"; suffix: " "; } @font-feature-values Font One { @styleset { nice-style: 12; } } .a { color: red; }\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class DeclarationAtRulesComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-at-sign-in-values',
    category: 'styles',
    description: 'An unquoted @ in a declaration value or at-rule header does not start an at-rule',
    className: 'AtSignInValuesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-at-sign-in-values',
  standalone: true,
  template: \`<div class="a b c">Content</div>\`,
  styles: [\`.a{background:url(a@2x.png)}.b{color:red}.c{color:blue}@supports (background: url(a@2x.png)) { .d{color:green} } .e{color:pink}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class AtSignInValuesComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },
  {
    name: 'shadow-css-combinator-spacing',
    category: 'styles',
    description:
      'Every combinator part is written as `${part} ${separator} `, so runs of spaces around combinators and deep combinators are kept',
    className: 'CombinatorSpacingComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-combinator-spacing',
  standalone: true,
  template: \`<div class="a b">Content</div>\`,
  styles: [\`.a  >  .b {} .c ~ .d, .e + .f {} :host-context(.x) ::ng-deep .y {} .g,.h , .i {}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class CombinatorSpacingComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-rule-body-strip',
    category: 'styles',
    description:
      'Angular 22.2 strips the host marker and deep combinators from a rule body without nested rules',
    className: 'RuleBodyStripComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-rule-body-strip',
  standalone: true,
  template: \`<div class="a b">Content</div>\`,
  styles: [\`.a { content: ":host.foo .bar"; color: red; } .b ::ng-deep .c { content: ">>> x"; }\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class RuleBodyStripComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },

  {
    name: 'shadow-css-nesting-selector',
    category: 'styles',
    description: 'Angular 22.2 leaves a lone & and a rule without a block unscoped',
    className: 'NestingSelectorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-shadow-css-nesting-selector',
  standalone: true,
  template: \`<div class="a">Content</div>\`,
  styles: [\`& .a {} .b & {} .c; .d {}\`],
  encapsulation: ViewEncapsulation.Emulated,
})
export class NestingSelectorComponent {}
    `.trim(),
    expectedFeatures: ['styles:'],
  },
]
