/**
 * Schema configuration fixtures.
 *
 * Tests the schemas option in @Component decorator:
 * - CUSTOM_ELEMENTS_SCHEMA: Allows use of custom HTML elements (web components)
 * - NO_ERRORS_SCHEMA: Suppresses all template validation errors
 *
 * Schemas control how Angular validates templates at compile time.
 * CUSTOM_ELEMENTS_SCHEMA is commonly used for:
 * - Web components integration
 * - Third-party custom elements
 * - Ionic/Capacitor components
 *
 * NO_ERRORS_SCHEMA is typically used for:
 * - Testing components in isolation
 * - Gradually migrating to Angular
 * - Stub components in unit tests
 *
 * NOTE: The schemas array appears in the component definition.
 * Values are:
 * - CUSTOM_ELEMENTS_SCHEMA = 1
 * - NO_ERRORS_SCHEMA = 2
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  // ==========================================================================
  // CUSTOM_ELEMENTS_SCHEMA
  // ==========================================================================

  {
    name: 'schemas-custom-elements-basic',
    category: 'schemas',
    description: 'Component with CUSTOM_ELEMENTS_SCHEMA for web components',
    className: 'CustomElementsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-custom-elements-basic',
  standalone: true,
  template: \`<my-custom-element></my-custom-element>\`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
})
export class CustomElementsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:'],
  },

  {
    name: 'schemas-custom-elements-with-properties',
    category: 'schemas',
    description: 'Custom element with property bindings',
    className: 'CustomElementPropertiesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-custom-elements-with-properties',
  standalone: true,
  template: \`
      <my-web-component
        [value]="data"
        [config]="configObject"
        (custom-event)="handleEvent($event)">
      </my-web-component>
    \`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
})
export class CustomElementPropertiesComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:', 'ɵɵproperty', 'ɵɵlistener'],
  },

  {
    name: 'schemas-custom-elements-with-attributes',
    category: 'schemas',
    description: 'Custom element with attribute bindings',
    className: 'CustomElementAttributesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-custom-elements-with-attributes',
  standalone: true,
  template: \`
      <custom-input
        [attr.data-id]="itemId"
        [attr.aria-label]="label"
        disabled>
      </custom-input>
    \`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
})
export class CustomElementAttributesComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:', 'ɵɵattribute'],
  },

  {
    name: 'schemas-custom-elements-nested',
    category: 'schemas',
    description: 'Nested custom elements',
    className: 'NestedCustomElementsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-custom-elements-nested',
  standalone: true,
  template: \`
      <custom-card>
        <custom-card-header>Title</custom-card-header>
        <custom-card-body>
          <custom-text>Content inside nested elements</custom-text>
        </custom-card-body>
        <custom-card-footer>Footer</custom-card-footer>
      </custom-card>
    \`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
})
export class NestedCustomElementsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:'],
  },

  {
    name: 'schemas-custom-elements-with-ngcontent',
    category: 'schemas',
    description: 'Custom elements with content projection',
    className: 'CustomElementsProjectionComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-custom-elements-with-ngcontent',
  standalone: true,
  template: \`
      <my-wrapper>
        <ng-content></ng-content>
        <custom-slot name="footer"></custom-slot>
      </my-wrapper>
    \`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
})
export class CustomElementsProjectionComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:', 'ɵɵprojection'],
  },

  {
    name: 'schemas-custom-elements-with-control-flow',
    category: 'schemas',
    description: 'Custom elements with Angular control flow',
    className: 'CustomElementsControlFlowComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-custom-elements-with-control-flow',
  standalone: true,
  template: \`
      @if (showElement) {
        <custom-dialog [open]="isOpen">
          @for (item of items; track item.id) {
            <custom-list-item [data]="item"></custom-list-item>
          }
        </custom-dialog>
      }
    \`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
})
export class CustomElementsControlFlowComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:', 'ɵɵconditional', 'ɵɵrepeaterCreate'],
  },

  {
    name: 'schemas-ionic-components',
    category: 'schemas',
    description: 'Ionic framework components (real-world use case)',
    className: 'IonicComponentsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-ionic-components',
  standalone: true,
  template: \`
      <ion-header>
        <ion-toolbar>
          <ion-title>{{ title }}</ion-title>
        </ion-toolbar>
      </ion-header>
      <ion-content>
        <ion-list>
          @for (item of items; track item.id) {
            <ion-item (click)="selectItem(item)">
              <ion-label>{{ item.name }}</ion-label>
            </ion-item>
          }
        </ion-list>
      </ion-content>
    \`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
})
export class IonicComponentsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:', 'ɵɵrepeaterCreate', 'ɵɵlistener'],
  },

  // ==========================================================================
  // NO_ERRORS_SCHEMA
  // ==========================================================================

  {
    name: 'schemas-no-errors-basic',
    category: 'schemas',
    description: 'Component with NO_ERRORS_SCHEMA',
    className: 'NoErrorsSchemaComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, NO_ERRORS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-no-errors-basic',
  standalone: true,
  template: \`<unknown-element attr="value"></unknown-element>\`,
  schemas: [NO_ERRORS_SCHEMA],
})
export class NoErrorsSchemaComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:'],
  },

  {
    name: 'schemas-no-errors-stub-components',
    category: 'schemas',
    description: 'Stub components in unit test scenario',
    className: 'StubComponentsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, NO_ERRORS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-no-errors-stub-components',
  standalone: true,
  template: \`
      <app-header [user]="currentUser"></app-header>
      <app-sidebar [menu]="menuItems"></app-sidebar>
      <app-content>{{ content }}</app-content>
      <app-footer></app-footer>
    \`,
  schemas: [NO_ERRORS_SCHEMA],
})
export class StubComponentsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:'],
  },

  {
    name: 'schemas-no-errors-unknown-attributes',
    category: 'schemas',
    description: 'Unknown attributes on elements',
    className: 'UnknownAttributesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, NO_ERRORS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-no-errors-unknown-attributes',
  standalone: true,
  template: \`
      <div unknown-directive="value" [unknownBinding]="data">
        <span (unknownEvent)="handler($event)">Content</span>
      </div>
    \`,
  schemas: [NO_ERRORS_SCHEMA],
})
export class UnknownAttributesComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:'],
  },

  {
    name: 'schemas-no-errors-mixed-elements',
    category: 'schemas',
    description: 'Mix of known and unknown elements',
    className: 'MixedElementsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, NO_ERRORS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-no-errors-mixed-elements',
  standalone: true,
  template: \`
      <div class="container">
        <unknown-header></unknown-header>
        <p>{{ message }}</p>
        <unknown-component [data]="data"></unknown-component>
        <button (click)="onClick()">Click me</button>
      </div>
    \`,
  schemas: [NO_ERRORS_SCHEMA],
})
export class MixedElementsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:', 'ɵɵlistener'],
  },

  // ==========================================================================
  // Both Schemas Combined
  // ==========================================================================

  {
    name: 'schemas-both-combined',
    category: 'schemas',
    description: 'Component with both schemas',
    className: 'BothSchemasComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA, NO_ERRORS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-both-combined',
  standalone: true,
  template: \`
      <custom-element></custom-element>
      <unknown-stub></unknown-stub>
      <div>{{ data }}</div>
    \`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA, NO_ERRORS_SCHEMA],
})
export class BothSchemasComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:'],
  },

  // ==========================================================================
  // Schemas with Other Component Metadata
  // ==========================================================================

  {
    name: 'schemas-with-providers',
    category: 'schemas',
    description: 'Schemas combined with providers',
    className: 'SchemasWithProvidersComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';

class DataService {}

@Component({
  selector: 'app-schemas-with-providers',
  standalone: true,
  template: \`
      <custom-data-viewer [data]="service.data"></custom-data-viewer>
    \`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
  providers: [DataService],
})
export class SchemasWithProvidersComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:', 'providers:'],
  },

  {
    name: 'schemas-with-encapsulation',
    category: 'schemas',
    description: 'Schemas with ViewEncapsulation.ShadowDom',
    className: 'SchemasWithShadowDomComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-schemas-with-encapsulation',
  standalone: true,
  template: \`
      <custom-shadowed-element>
        <slot name="content"></slot>
      </custom-shadowed-element>
    \`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
  encapsulation: ViewEncapsulation.ShadowDom,
})
export class SchemasWithShadowDomComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:', 'encapsulation: 3'],
  },

  {
    name: 'schemas-with-change-detection',
    ngtscMode: 'full',
    category: 'schemas',
    description: 'Schemas with OnPush change detection',
    className: 'SchemasWithOnPushComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA, ChangeDetectionStrategy } from '@angular/core';

@Component({
  selector: 'app-schemas-with-change-detection',
  standalone: true,
  template: \`
      <custom-async-component [data$]="data$"></custom-async-component>
    \`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class SchemasWithOnPushComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:', 'ChangeDetectionStrategy.OnPush'],
  },

  {
    name: 'schemas-with-host-bindings',
    category: 'schemas',
    description: 'Schemas with host bindings',
    className: 'SchemasWithHostBindingsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-with-host-bindings',
  standalone: true,
  template: \`<custom-styled-element></custom-styled-element>\`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
  host: {
    '[class.custom-host]': 'isCustom',
    '[attr.data-component]': 'componentType',
  },
})
export class SchemasWithHostBindingsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:'],
  },

  // ==========================================================================
  // Real-World Integration Scenarios
  // ==========================================================================

  {
    name: 'schemas-material-web',
    category: 'schemas',
    description: 'Material Web Components integration',
    className: 'MaterialWebComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-material-web',
  standalone: true,
  template: \`
      <md-filled-button (click)="save()">Save</md-filled-button>
      <md-outlined-text-field
        [label]="fieldLabel"
        [value]="inputValue"
        (input)="onInput($event)">
      </md-outlined-text-field>
      <md-dialog [open]="dialogOpen">
        <div slot="headline">Dialog Title</div>
        <div slot="content">{{ dialogContent }}</div>
        <div slot="actions">
          <md-text-button (click)="closeDialog()">Cancel</md-text-button>
          <md-filled-button (click)="confirm()">Confirm</md-filled-button>
        </div>
      </md-dialog>
    \`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
})
export class MaterialWebComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:', 'ɵɵlistener'],
  },

  {
    name: 'schemas-stencil-components',
    category: 'schemas',
    description: 'Stencil.js web components',
    className: 'StencilComponentsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, CUSTOM_ELEMENTS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-stencil-components',
  standalone: true,
  template: \`
      <my-component first="first" [middle]="middleName" [last]="lastName"></my-component>
      @for (item of items; track item.id) {
        <my-item-component
          [item]="item"
          (itemSelected)="onSelect($event)">
        </my-item-component>
      }
    \`,
  schemas: [CUSTOM_ELEMENTS_SCHEMA],
})
export class StencilComponentsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:', 'ɵɵrepeaterCreate', 'ɵɵlistener'],
  },

  {
    name: 'schemas-testing-scenario',
    category: 'schemas',
    description: 'Unit testing scenario with shallow rendering',
    className: 'TestedComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, NO_ERRORS_SCHEMA } from '@angular/core';

@Component({
  selector: 'app-schemas-testing-scenario',
  standalone: true,
  template: \`
      <app-header [title]="pageTitle"></app-header>
      <app-navigation [items]="navItems" (navigate)="onNavigate($event)"></app-navigation>
      <main>
        <app-content-area>
          {{ mainContent }}
        </app-content-area>
      </main>
      <app-footer [copyright]="copyrightYear"></app-footer>
    \`,
  schemas: [NO_ERRORS_SCHEMA],
})
export class TestedComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'schemas:'],
  },
]
