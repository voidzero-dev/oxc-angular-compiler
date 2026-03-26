/**
 * @for with various track expressions.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'for-track-property',
    category: 'control-flow',
    description: '@for with track by property',
    className: 'ForTrackPropertyComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-for-track-property',
  standalone: true,
  template: \`
    @for (item of items; track item.id) {
      <div>{{ item.name }}</div>
    }
  \`,
})
export class ForTrackPropertyComponent {
  items = [{ id: 1, name: 'Item 1' }, { id: 2, name: 'Item 2' }];
}
    `.trim(),
    expectedFeatures: ['ɵɵrepeaterCreate', 'ɵɵrepeater'],
  },
  {
    name: 'for-track-index',
    category: 'control-flow',
    description: '@for with track by $index',
    className: 'ForTrackIndexComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-for-track-index',
  standalone: true,
  template: \`
    @for (item of items; track $index) {
      <div>{{ item }}</div>
    }
  \`,
})
export class ForTrackIndexComponent {
  items = ['a', 'b', 'c'];
}
    `.trim(),
    expectedFeatures: ['ɵɵrepeaterCreate'],
  },
  {
    name: 'for-track-expression',
    category: 'control-flow',
    description: '@for with complex track expression',
    className: 'ForTrackExpressionComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-for-track-expression',
  standalone: true,
  template: \`
    @for (item of items; track item.category + item.id) {
      <div>{{ item.name }}</div>
    }
  \`,
})
export class ForTrackExpressionComponent {
  items = [{ id: 1, category: 'A', name: 'Item 1' }];
}
    `.trim(),
    expectedFeatures: ['ɵɵrepeaterCreate'],
  },
  {
    name: 'for-with-empty',
    category: 'control-flow',
    description: '@for with @empty block',
    className: 'ForWithEmptyComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-for-with-empty',
  standalone: true,
  template: \`
    @for (item of items; track item.id) {
      <div>{{ item.name }}</div>
    } @empty {
      <div>No items found</div>
    }
  \`,
})
export class ForWithEmptyComponent {
  items: { id: number; name: string }[] = [];
}
    `.trim(),
    expectedFeatures: ['ɵɵrepeaterCreate', 'ɵɵrepeaterTrackByIdentity'],
  },
  {
    name: 'for-track-binary-with-component-method',
    category: 'control-flow',
    description: '@for with track using binary operator and component method',
    className: 'ForTrackBinaryComponentMethodComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-for-track-binary-method',
  standalone: true,
  template: \`
    @for (item of items; track prefix() + item.id) {
      <div>{{ item.name }}</div>
    }
  \`,
})
export class ForTrackBinaryComponentMethodComponent {
  items = [{ id: '1', name: 'Item 1' }];
  prefix() { return 'pfx'; }
}
    `.trim(),
    expectedFeatures: ['ɵɵrepeaterCreate'],
  },
  {
    name: 'for-track-nullish-coalescing-with-component-method',
    category: 'control-flow',
    description: '@for with track using ?? operator and component method',
    className: 'ForTrackNullishCoalesceComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-for-track-nullish',
  standalone: true,
  template: \`
    @for (tag of tags; track (tag.queryPrefix ?? queryPrefix()) + '.' + tag.key) {
      <span>{{ tag.key }}</span>
    }
  \`,
})
export class ForTrackNullishCoalesceComponent {
  tags = [{ queryPrefix: null, key: 'k1' }];
  queryPrefix() { return 'default'; }
}
    `.trim(),
    expectedFeatures: ['ɵɵrepeaterCreate'],
  },
  {
    name: 'for-track-ternary-with-component-method',
    category: 'control-flow',
    description: '@for with track using ternary and component method',
    className: 'ForTrackTernaryComponentMethodComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-for-track-ternary-method',
  standalone: true,
  template: \`
    @for (item of items; track useId() ? item.id : item.name) {
      <div>{{ item.name }}</div>
    }
  \`,
})
export class ForTrackTernaryComponentMethodComponent {
  items = [{ id: '1', name: 'Item 1' }];
  useId() { return true; }
}
    `.trim(),
    expectedFeatures: ['ɵɵrepeaterCreate'],
  },
  {
    name: 'for-context-variables',
    category: 'control-flow',
    description: '@for with all context variables',
    className: 'ForContextVariablesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-for-context-variables',
  standalone: true,
  template: \`
    @for (item of items; track item.id; let idx = $index, first = $first, last = $last, even = $even, odd = $odd, count = $count) {
      <div [class.first]="first" [class.last]="last" [class.even]="even" [class.odd]="odd">
        {{ idx + 1 }}/{{ count }}: {{ item.name }}
      </div>
    }
  \`,
})
export class ForContextVariablesComponent {
  items = [{ id: 1, name: 'Item 1' }, { id: 2, name: 'Item 2' }, { id: 3, name: 'Item 3' }];
}
    `.trim(),
    expectedFeatures: ['ɵɵrepeaterCreate'],
  },
]
