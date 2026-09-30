import { describe, expect, it } from 'vitest'

import { extractComponentMetadataSync } from '../index.js'

// The queries reported must be the ones the component is compiled with.
describe('extractComponentMetadataSync queries', () => {
  it('resolves const selectors and includes `queries:` metadata after member queries', () => {
    const [component] = extractComponentMetadataSync(
      `
import { Component, ContentChild, ViewChild } from '@angular/core';

const SEL = 'ref';

@Component({
  selector: 'app-x',
  template: '<div #ref></div>',
  queries: {
    fromMetaView: new ViewChild('metaRef'),
    fromMetaContent: new ContentChild('metaContent', { descendants: false }),
  },
})
export class X {
  @ViewChild(SEL) member: any;
  @ContentChild('memberContent') memberContent: any;
}
`,
      'x.component.ts',
    )

    expect(component.viewQueries?.map((q) => [q.propertyName, q.predicate])).toEqual([
      ['member', '["ref"]'],
      ['fromMetaView', '["metaRef"]'],
    ])
    expect(component.queries?.map((q) => [q.propertyName, q.predicate, q.descendants])).toEqual([
      ['memberContent', '["memberContent"]', true],
      ['fromMetaContent', '["metaContent"]', false],
    ])
  })
})

// ngtsc reads `@NS.Input()` and the other member decorators only when `NS` is a
// namespace import of `@angular/core`.
describe('extractComponentMetadataSync namespaced member decorators', () => {
  const source = (module: string) => `
import { Component } from '@angular/core';
import * as NS from '${module}';

@Component({ selector: 'app-x', template: '<div #ref></div>' })
export class X {
  @NS.Input() value!: string;
  @NS.Output() changed: any;
  @NS.ViewChild('ref') ref: any;
  @NS.HostListener('click') onClick() {}
}
`

  it('reads them through a namespace import of @angular/core', () => {
    const [component] = extractComponentMetadataSync(source('@angular/core'), 'x.component.ts')
    expect(component.inputs?.map((i) => i.classPropertyName)).toEqual(['value'])
    expect(component.outputs?.map((o) => o.classPropertyName)).toEqual(['changed'])
    expect(component.viewQueries?.map((q) => q.propertyName)).toEqual(['ref'])
    expect(component.host?.listeners).toEqual([['(click)', 'onClick()']])
  })

  it('ignores them through any other namespace', () => {
    const [component] = extractComponentMetadataSync(source('foreign-decorators'), 'x.component.ts')
    expect(component.inputs ?? []).toEqual([])
    expect(component.outputs ?? []).toEqual([])
    expect(component.viewQueries ?? []).toEqual([])
    expect(component.host?.listeners ?? []).toEqual([])
  })
})
