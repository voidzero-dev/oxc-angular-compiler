/**
 * Regression: `ɵɵi18nPostprocess` must use namespace prefix `i0.ɵɵi18nPostprocess`.
 *
 * ## The Issue
 *
 * `wrap_with_postprocess()` in i18n_const_collection.rs used a bare
 * `ReadVar(ɵɵi18nPostprocess)` which emitted `ɵɵi18nPostprocess(...)` without
 * the `i0.` namespace prefix. At runtime:
 * `ReferenceError: ɵɵi18nPostprocess is not defined`.
 *
 * ## Root Cause
 *
 * The function was constructing `OutputExpression::ReadVar` for the runtime helper,
 * but all Angular runtime functions must be accessed through the namespace import
 * (`i0.ɵɵ...`). The correct expression is `OutputExpression::ReadProp` with
 * `i0` as the receiver.
 *
 * ## Fix
 *
 * Changed to `ReadProp(i0.ɵɵi18nPostprocess)` matching all other Angular
 * runtime calls.
 *
 * ## Impact
 *
 * RUNTIME_AFFECTING: Any component with nested ICU expressions (plural/select with
 * sub-messages) would crash with ReferenceError. Discovered in ClickUp's
 * ChatBotTriggerComponent which uses plural with HTML elements in branches.
 *
 * ## When ɵɵi18nPostprocess is triggered
 *
 * The postprocess function is called when ICU messages contain:
 * - Nested plural/select expressions with sub-messages
 * - HTML elements inside ICU branches (e.g., `<strong>{{ name }}</strong>`)
 * - Multiple sub-expressions that need placeholder replacement
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    type: 'full-transform',
    name: 'i18n-postprocess-namespace-nested-icu',
    category: 'regressions',
    description: 'Nested ICU with HTML elements should use i0.ɵɵi18nPostprocess',
    className: 'ChatBotTriggerComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-chatbot',
  standalone: true,
  template: \`<span i18n>{count, plural, =1 {<strong>{{ name }}</strong> item} other {{{ count }} items}}</span>\`,
})
export class ChatBotTriggerComponent {
  count = 0;
  name = '';
}
`.trim(),
    expectedFeatures: ['ɵɵi18n', 'ɵɵi18nExp'],
  },
  {
    type: 'full-transform',
    name: 'i18n-postprocess-namespace-nested-select',
    category: 'regressions',
    description: 'Nested select ICU should use namespace-prefixed postprocess',
    className: 'UndoToastComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-undo-toast',
  standalone: true,
  template: \`<span i18n>{count, plural,
    =1 {<strong>{{ name }}</strong> was deleted from {nestedCount, plural,
      =1 {<strong>{{ category }}</strong>}
      other {<strong>{{ category }}</strong> and {{ extra }} more}
    }}
    other {{{ count }} items deleted}
  }</span>\`,
})
export class UndoToastComponent {
  count = 0;
  name = '';
  nestedCount = 0;
  category = '';
  extra = 0;
}
`.trim(),
    expectedFeatures: ['ɵɵi18n', 'ɵɵi18nExp'],
  },
]
