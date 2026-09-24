/**
 * `inputs:` / `outputs:` in decorator metadata, ported from Angular's own tests
 * (angular/angular v22.1.7). Acceptance-spec sources are made standalone
 * (`imports: [Dir]`) in place of their TestBed NgModule, and drop the TestBed-only
 * `changeDetection: Eager`; directive metadata is verbatim.
 */
import type { Fixture } from '../types.js'

const fixture = (
  name: string,
  description: string,
  className: string,
  sourceCode: string,
): Fixture => ({
  type: 'full-transform',
  name: `decorator-metadata-${name}`,
  category: 'inputs-outputs',
  description,
  className,
  sourceCode: sourceCode.trim(),
})

export const fixtures: Fixture[] = [
  // compiler-cli/test/ngtsc/ngtsc_spec.ts: 'should allow directives with no selector that are not in NgModules'
  fixture(
    'no-selector',
    'selectorless directive with an inputs array',
    'TestDirWithInputs',
    `
import {Directive} from '@angular/core';

@Directive({})
export class BaseDir {}

@Directive({})
export abstract class AbstractBaseDir {}

@Directive()
export abstract class EmptyDir {}

@Directive({
  inputs: ['a', 'b'],
  standalone: false,
})
export class TestDirWithInputs {}
`,
  ),
  // ngtsc_spec.ts: 'should wrap "inputs" and "outputs" keys if they contain unsafe characters'
  fixture(
    'unsafe-keys',
    'inputs/outputs names that need quoting, alongside @Input members',
    'SomeDir',
    `
import {Directive, Input} from '@angular/core';

@Directive({
  selector: '[somedir]',
  inputs: ['track-type', 'track-name', 'inputTrackName', 'src.xl'],
  outputs: ['output-track-type', 'output-track-name', 'outputTrackName', 'output.event']
})
export class SomeDir {
  @Input('track-type') trackType: string;
  @Input('track-name') trackName: string;
}
`,
  ),
  // ngtsc_spec.ts: 'should generate the correct declaration for directives using the `inputs` array'
  fixture(
    'all-forms',
    'every string and object form of the inputs array',
    'TestDir',
    `
import {Directive, Input} from '@angular/core';

@Directive({
  selector: '[dir]',
  inputs: [
    'plain',
    'withAlias: aliasedWithAlias',
    {name: 'plainLiteral'},
    {name: 'aliasedLiteral', alias: 'alisedLiteralAlias'},
    {name: 'requiredLiteral', required: true},
    {name: 'requiredAlisedLiteral', alias: 'requiredAlisedLiteralAlias', required: true}
  ],
  standalone: false,
})
export class TestDir {
  plainLiteral: any;
  aliasedLiteral: any;
  requiredLiteral: any;
  requiredAlisedLiteral: any;
}
`,
  ),
  // ngtsc_spec.ts: 'should *not* generate a validator fn for attribute and property bindings when *not* on <iframe>'
  fixture(
    'sandbox-binding',
    'template binding matched against a metadata-declared input',
    'SomeComponent',
    `
import {Component, Directive} from '@angular/core';

@Directive({
  selector: '[sandbox]',
  inputs: ['sandbox']
})
class Dir {}

@Component({
  imports: [Dir],
  template: \`
    <div [sandbox]="''" [title]="'Hi!'"></div>
  \`
})
export class SomeComponent {}
`,
  ),
  // compiler-cli/test/compliance/test_cases/r3_view_compiler_bindings/order_bindings.ts
  fixture(
    'order-bindings',
    'component inputs array consumed by a parent template',
    'MyCmp',
    `
import {Component} from '@angular/core';

@Component({
  selector: 'some-elem',
  template: \`\`,
  inputs: ['attr1', 'prop1', 'attrInterp1', 'propInterp1'],
})
export class SomeCmp {}

@Component({
  selector: 'my-cmp',
  imports: [SomeCmp],
  host: {
    'literal1': 'foo',
    '(event1)': 'foo()',
    '[attr.attr1]': 'foo',
    '[id]': 'foo',
    '[class.class1]': 'false',
    '[style.style1]': 'true',
    '[class]': 'foo',
    '[style]': 'foo',
  },
  template: \`
		<some-elem
			literal1="foo"
			(event1)="foo()"
			[attr.attr1]="foo"
			[prop1]="foo",
			[class.class1]="foo",
			[style.style1]="foo"
			style="foo"
			class="foo"
			attr.attrInterp1="interp {{foo}}"
			propInterp1="interp {{foo}}"
			/>
	\`,
})
export class MyCmp {
  foo: any;
}
`,
  ),
  // core/test/acceptance/directive_spec.ts: '... object literal syntax in the `inputs` array'
  fixture(
    'object-literal',
    'object literal entries in the inputs array',
    'App',
    `
import {Component, Directive, ViewChild} from '@angular/core';

@Directive({
  selector: '[dir]',
  inputs: [{name: 'plainInput'}, {name: 'aliasedInput', alias: 'alias'}],
})
class Dir {
  plainInput: number | undefined;
  aliasedInput: number | undefined;
}

@Component({
  imports: [Dir],
  template: '<div dir [plainInput]="plainValue" [alias]="aliasedValue"></div>',
})
export class App {
  @ViewChild(Dir) dirInstance!: Dir;
  plainValue = 123;
  aliasedValue = 321;
}
`,
  ),
  // directive_spec.ts: 'should transform incoming input values when declared through the `inputs` array'
  fixture(
    'inline-transform',
    'inline arrow transform in the inputs array',
    'TestComp',
    `
import {Component, Directive, ViewChild} from '@angular/core';

@Directive({
  selector: '[dir]',
  inputs: [{name: 'value', transform: (value: string) => (value ? 1 : 0)}],
})
class Dir {
  value = -1;
}

@Component({
  imports: [Dir],
  template: '<div dir [value]="assignedValue"></div>',
})
export class TestComp {
  @ViewChild(Dir) dir!: Dir;
  assignedValue = '';
}
`,
  ),
  // core/test/acceptance/inherit_definition_feature_spec.ts: 'should not inherit transforms if inputs are re-declared'
  // The spec runs under JIT; AOT rejects its untyped transform parameter (see
  // crates/oxc_angular_compiler/tests/decorator_metadata_ngtsc_test.rs), so it's typed here.
  fixture(
    'redeclared-inherited',
    'subclass re-declares a base @Input through the inputs array',
    'TestCmp',
    `
import {Component, Directive, Input} from '@angular/core';

@Directive()
class Base {
  @Input({transform: (v: string) => \`\${v}-transformed\`}) someInput: string = '';
}

@Directive({
  selector: 'dir',
  inputs: ['someInput'],
})
class ActualDir extends Base {}

@Component({
  imports: [ActualDir],
  template: \`<dir someInput="newValue"></dir>\`,
})
export class TestCmp {}
`,
  ),
]
