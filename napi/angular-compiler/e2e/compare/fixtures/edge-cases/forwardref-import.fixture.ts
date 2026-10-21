/**
 * forwardRef in component imports.
 *
 * Tests components that use forwardRef() in their imports array to handle
 * circular dependency scenarios where a component needs to import another
 * component that is declared later in the file or in a circular module.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'forwardref-basic-import',
    // The components it imports have no imports of their own and compile like a full build.
    ngtscMode: 'full',
    category: 'edge-cases',
    description: 'Basic forwardRef in imports array',
    className: 'ForwardRefBasicImportComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, forwardRef } from '@angular/core';

@Component({
  selector: 'app-forward-ref-basic',
  standalone: true,
  imports: [forwardRef(() => ChildComponent)],
  template: \`<child-component></child-component>\`,
})
export class ForwardRefBasicImportComponent {}

@Component({
  selector: 'child-component',
  standalone: true,
  template: \`<span>Child</span>\`,
})
export class ChildComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵelement'],
  },
  {
    name: 'forwardref-multiple-imports',
    // The components it imports have no imports of their own and compile like a full build.
    ngtscMode: 'full',
    category: 'edge-cases',
    description: 'Multiple forwardRef in imports',
    className: 'ForwardRefMultipleImportsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, forwardRef } from '@angular/core';

@Component({
  selector: 'app-forward-ref-multiple',
  standalone: true,
  imports: [
    forwardRef(() => ComponentA),
    forwardRef(() => ComponentB),
    forwardRef(() => ComponentC),
  ],
  template: \`
    <component-a></component-a>
    <component-b></component-b>
    <component-c></component-c>
  \`,
})
export class ForwardRefMultipleImportsComponent {}

@Component({ selector: 'component-a', standalone: true, template: '<span>A</span>' })
export class ComponentA {}

@Component({ selector: 'component-b', standalone: true, template: '<span>B</span>' })
export class ComponentB {}

@Component({ selector: 'component-c', standalone: true, template: '<span>C</span>' })
export class ComponentC {}
    `.trim(),
    expectedFeatures: ['ɵɵelement'],
  },
  {
    name: 'forwardref-mixed-imports',
    // The components it imports have no imports of their own and compile like a full build.
    ngtscMode: 'full',
    category: 'edge-cases',
    description: 'Mix of regular and forwardRef imports',
    className: 'ForwardRefMixedImportsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, forwardRef } from '@angular/core';
import { CommonModule } from '@angular/common';

@Component({
  selector: 'app-forward-ref-mixed',
  standalone: true,
  imports: [
    CommonModule,
    forwardRef(() => LazyComponent),
  ],
  template: \`
    <div *ngIf="show">
      <lazy-component></lazy-component>
    </div>
  \`,
})
export class ForwardRefMixedImportsComponent {
  show = true;
}

@Component({
  selector: 'lazy-component',
  standalone: true,
  template: '<span>Lazy loaded via forwardRef</span>',
})
export class LazyComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵtemplate'],
  },
  {
    name: 'forwardref-recursive-component',
    category: 'edge-cases',
    description: 'Recursive component using forwardRef',
    className: 'TreeNodeComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, forwardRef, Input } from '@angular/core';

@Component({
  selector: 'app-tree-node',
  standalone: true,
  imports: [forwardRef(() => TreeNodeComponent)],
  template: \`
    <div class="node">
      <span>{{ node.label }}</span>
      @if (node.children) {
        @for (child of node.children; track child.id) {
          <app-tree-node [node]="child"></app-tree-node>
        }
      }
    </div>
  \`,
})
export class TreeNodeComponent {
  @Input() node: any;
}
    `.trim(),
    expectedFeatures: ['ɵɵconditional', 'ɵɵrepeaterCreate', 'ɵɵelement'],
  },
  {
    name: 'forwardref-with-template',
    // The components it imports have no imports of their own and compile like a full build.
    ngtscMode: 'full',
    category: 'edge-cases',
    description: 'forwardRef with multiple lazy-loaded components',
    className: 'ForwardRefWithTemplateComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, forwardRef } from '@angular/core';

@Component({
  selector: 'app-forward-ref-with-template',
  standalone: true,
  imports: [
    forwardRef(() => SelfReferencingItemComponent),
    forwardRef(() => OtherLazyItemComponent),
  ],
  template: \`
    <self-referencing-item></self-referencing-item>
    <other-lazy-item></other-lazy-item>
  \`,
})
export class ForwardRefWithTemplateComponent {}

@Component({
  selector: 'self-referencing-item',
  standalone: true,
  template: '<span>Self Referencing</span>',
})
export class SelfReferencingItemComponent {}

@Component({
  selector: 'other-lazy-item',
  standalone: true,
  template: '<span>Other Lazy</span>',
})
export class OtherLazyItemComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵelement'],
  },
]
