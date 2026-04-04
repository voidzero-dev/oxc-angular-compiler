/**
 * Class field declarations with parameter properties.
 *
 * Tests how non-Angular classes with TypeScript parameter properties are handled.
 * When useDefineForClassFields: true (default for ES2022+), TypeScript emits
 * explicit field declarations, but these are cosmetic differences that don't
 * affect runtime behavior.
 *
 * This fixture documents the known difference between OXC and TypeScript output
 * for helper classes that use parameter properties.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'parameter-property-class',
    category: 'edge-cases',
    description: 'Non-Angular class with TypeScript parameter properties',
    className: 'ParameterPropertyComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

/**
 * Helper class with parameter properties.
 * TypeScript (ES2022+) emits: class Foo { x; constructor(x) { this.x = x; } }
 * OXC emits: class Foo { constructor(x) { this.x = x; } }
 * Both are functionally equivalent at runtime.
 */
class DataNode {
  constructor(
    public id: string,
    public name: string,
    public level = 0,
    public expanded = false,
  ) {}
}

@Component({
  selector: 'app-parameter-property',
  standalone: true,
  template: \`<span>{{ node?.name }}</span>\`,
})
export class ParameterPropertyComponent {
  node = new DataNode('1', 'Test', 0, false);
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
    skip: true,
    skipReason:
      'Known cosmetic difference: TypeScript emits explicit field declarations for parameter properties with useDefineForClassFields:true, OXC does not. Both are functionally equivalent.',
  },
  {
    name: 'class-field-initializer',
    category: 'edge-cases',
    description: 'Non-Angular class with class field initializer',
    className: 'ClassFieldInitializerComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { BehaviorSubject } from 'rxjs';

/**
 * Helper class with class field initializer and getter.
 * The childrenChange field should be preserved identically.
 */
class NodeWithChildren {
  childrenChange = new BehaviorSubject<string[]>([]);

  get children(): string[] {
    return this.childrenChange.value;
  }

  constructor(
    public name: string,
    public hasChildren = false,
  ) {}
}

@Component({
  selector: 'app-class-field-init',
  standalone: true,
  template: \`<span>{{ node?.name }}</span>\`,
})
export class ClassFieldInitializerComponent {
  node = new NodeWithChildren('Root', true);
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
    skip: true,
    skipReason:
      'Known cosmetic difference: TypeScript emits explicit field declarations for parameter properties with useDefineForClassFields:true, OXC does not. Both are functionally equivalent.',
  },
]
