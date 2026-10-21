/**
 * `hostDirectives` as ngtsc reads it: through the partial evaluator (constants, spreads,
 * `forwardRef`), with the shorthand `HostDirectivesFeature` argument rules of
 * `createHostDirectivesFeatureArg`.
 *
 * - An entry without `inputs` and `outputs` is passed as the bare class; once either key is
 *   written (even as an empty array) it is passed as `{ directive }`.
 * - A `forwardRef` entry makes the whole argument a `function () { return [...] }`.
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
    category: 'host-directives',
    description,
    className,
    type: 'full-transform',
    sourceCode: sourceCode.trim(),
    expectedFeatures: ['ɵɵHostDirectivesFeature'],
  }
}

export const fixtures: Fixture[] = [
  fixture(
    'host-directives-object-without-mappings',
    'An object entry with only `directive` is passed as the bare class',
    'ObjectNoMappingsComponent',
    `
import { Component, Directive } from '@angular/core';

@Directive({ selector: '[basic]' })
export class BasicDirective {}

@Component({
  selector: 'app-object-no-mappings',
  hostDirectives: [{ directive: BasicDirective }],
  template: '',
})
export class ObjectNoMappingsComponent {}
`,
  ),
  fixture(
    'host-directives-empty-outputs-only',
    'An empty `outputs` alone still gives `{ directive }`',
    'EmptyOutputsComponent',
    `
import { Component, Directive } from '@angular/core';

@Directive({ selector: '[basic]' })
export class BasicDirective {}

@Component({
  selector: 'app-empty-outputs',
  hostDirectives: [BasicDirective, { directive: BasicDirective, outputs: [] }],
  template: '',
})
export class EmptyOutputsComponent {}
`,
  ),
  fixture(
    'directive-host-directives-shorthand',
    'A directive passes a host directive without mappings as the bare class',
    'ShorthandHostDirective',
    `
import { Directive } from '@angular/core';

@Directive({ selector: '[basic]' })
export class BasicDirective {}

@Directive({
  selector: '[shorthand]',
  hostDirectives: [BasicDirective, { directive: BasicDirective }],
})
export class ShorthandHostDirective {}
`,
  ),
  fixture(
    'directive-host-directives-empty-mappings',
    'A directive passes a host directive with empty mappings as `{ directive }`',
    'EmptyMappingsHostDirective',
    `
import { Directive } from '@angular/core';

@Directive({ selector: '[basic]' })
export class BasicDirective {}

@Directive({
  selector: '[emptyMappings]',
  hostDirectives: [{ directive: BasicDirective, inputs: [], outputs: [] }],
})
export class EmptyMappingsHostDirective {}
`,
  ),
  fixture(
    'host-directives-forward-ref',
    'A forwardRef entry wraps the whole argument in a function',
    'ForwardRefHostComponent',
    `
import { Component, Directive, forwardRef } from '@angular/core';

@Directive({ selector: '[early]' })
export class EarlyDirective {}

@Component({
  selector: 'app-forward-ref-host',
  hostDirectives: [EarlyDirective, forwardRef(() => LaterDirective)],
  template: '',
})
export class ForwardRefHostComponent {}

@Directive({ selector: '[later]' })
export class LaterDirective {}
`,
  ),
  fixture(
    'host-directives-forward-ref-object',
    'A forwardRef `directive` in an object entry with mappings',
    'ForwardRefObjectComponent',
    `
import { Component, Directive, Input, forwardRef } from '@angular/core';

@Component({
  selector: 'app-forward-ref-object',
  hostDirectives: [{ directive: forwardRef(() => LaterDirective), inputs: ['value: alias'] }],
  template: '',
})
export class ForwardRefObjectComponent {}

@Directive({ selector: '[later]' })
export class LaterDirective {
  @Input() value = '';
}
`,
  ),
  fixture(
    'directive-host-directives-forward-ref',
    'A directive with a forwardRef host directive wraps the whole argument in a function',
    'ForwardRefHostDirective',
    `
import { Directive, forwardRef } from '@angular/core';

@Directive({
  selector: '[forwardRefHost]',
  hostDirectives: [forwardRef(() => LaterDirective)],
})
export class ForwardRefHostDirective {}

@Directive({ selector: '[later]' })
export class LaterDirective {}
`,
  ),
  fixture(
    'host-directives-const-array',
    'hostDirectives read from a constant',
    'ConstHostDirectivesComponent',
    `
import { Component, Directive } from '@angular/core';

@Directive({ selector: '[a]' })
export class ADirective {}

@Directive({ selector: '[b]' })
export class BDirective {}

const HOST_DIRECTIVES = [ADirective, { directive: BDirective, inputs: [] }];

@Component({
  selector: 'app-const-host-directives',
  hostDirectives: HOST_DIRECTIVES,
  template: '',
})
export class ConstHostDirectivesComponent {}
`,
  ),
  fixture(
    'host-directives-spread',
    'hostDirectives with a spread constant',
    'SpreadHostDirectivesComponent',
    `
import { Component, Directive } from '@angular/core';

@Directive({ selector: '[a]' })
export class ADirective {}

@Directive({ selector: '[b]' })
export class BDirective {}

const SHARED = [ADirective];

@Component({
  selector: 'app-spread-host-directives',
  hostDirectives: [...SHARED, BDirective],
  template: '',
})
export class SpreadHostDirectivesComponent {}
`,
  ),
  fixture(
    'directive-host-directives-const-mappings',
    'Host directive mappings and the directive read from constants',
    'ConstMappingsHostDirective',
    `
import { Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[base]' })
export class BaseDirective {
  @Input() value = '';
  @Output() changed = new EventEmitter<string>();
}

const BASE = BaseDirective;
const INPUTS = ['value: alias'];
const OUTPUT = 'changed';

@Directive({
  selector: '[constMappings]',
  hostDirectives: [{ directive: BASE, inputs: INPUTS, outputs: [OUTPUT] }],
})
export class ConstMappingsHostDirective {}
`,
  ),
  fixture(
    'host-directives-duplicate-mapping',
    'A field mapped twice keeps one entry, the last alias, at its first position',
    'DuplicateMappingComponent',
    `
import { Component, Directive, Input } from '@angular/core';

@Directive({ selector: '[base]' })
export class BaseDirective {
  @Input() a = '';
  @Input() b = '';
}

@Component({
  selector: 'app-duplicate-mapping',
  hostDirectives: [{ directive: BaseDirective, inputs: ['a: x', 'b', 'a: y'] }],
  template: '',
})
export class DuplicateMappingComponent {}
`,
  ),
]
