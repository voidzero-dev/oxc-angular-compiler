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
