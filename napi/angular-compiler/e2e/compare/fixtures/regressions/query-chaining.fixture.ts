/**
 * Regression: Multiple queries must emit separate statements, not chained calls.
 *
 * ## The Issue
 *
 * Multiple `@ViewChild`/`@ViewChildren`/`@ContentChild`/`@ContentChildren` queries
 * were chained as `ɵɵviewQuery(pred1)(pred2)`, treating the return value of
 * `ɵɵviewQuery` as a callable. Angular 20's `ɵɵviewQuery` returns `void`, so
 * chaining causes: `TypeError: i0.ɵɵviewQuery(...) is not a function`.
 *
 * ## Root Cause
 *
 * `create_view_queries_function` and `create_content_queries_function` in
 * directive/query.rs built a chain of calls by wrapping each new query call around
 * the previous one's return value.
 *
 * ## Fix
 *
 * Emit each query as a separate statement:
 *   `ɵɵviewQuery(pred1); ɵɵviewQuery(pred2);`
 *
 * ## Impact
 *
 * RUNTIME_AFFECTING: Any component with 2+ view queries or 2+ content queries
 * would crash at bootstrap. Discovered in ClickUp's LoginFormComponent.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    type: 'full-transform',
    name: 'query-chaining-multiple-viewchild',
    category: 'regressions',
    description: 'Multiple @ViewChild should emit separate ɵɵviewQuery statements',
    className: 'LoginFormComponent',
    sourceCode: `
import { Component, ViewChild, ElementRef } from '@angular/core';

@Component({
  selector: 'app-login',
  standalone: true,
  template: '<input #emailInput /><input #passwordInput /><button #submitBtn>Login</button>',
})
export class LoginFormComponent {
  @ViewChild('emailInput') emailInput!: ElementRef;
  @ViewChild('passwordInput') passwordInput!: ElementRef;
  @ViewChild('submitBtn') submitBtn!: ElementRef;
}
`.trim(),
    expectedFeatures: ['ɵɵviewQuery', 'ɵɵqueryRefresh', 'ɵɵloadQuery'],
  },
  {
    type: 'full-transform',
    name: 'query-chaining-multiple-contentchild',
    category: 'regressions',
    description:
      'Multiple @ContentChild/@ContentChildren should emit separate ɵɵcontentQuery statements',
    className: 'TabsComponent',
    sourceCode: `
import { Component, ContentChild, ContentChildren, QueryList, TemplateRef } from '@angular/core';

@Component({
  selector: 'app-tabs',
  standalone: true,
  template: '<ng-content></ng-content>',
})
export class TabsComponent {
  @ContentChild('header') header!: TemplateRef<any>;
  @ContentChildren('tab') tabs!: QueryList<TemplateRef<any>>;
  @ContentChild('footer') footer!: TemplateRef<any>;
}
`.trim(),
    expectedFeatures: ['ɵɵcontentQuery', 'ɵɵqueryRefresh', 'ɵɵloadQuery'],
  },
  {
    type: 'full-transform',
    name: 'query-chaining-mixed-view-and-content',
    category: 'regressions',
    description: 'Mixed ViewChild and ContentChild queries should all emit separate statements',
    className: 'MixedQueryComponent',
    sourceCode: `
import { Component, ViewChild, ContentChild, ElementRef, TemplateRef } from '@angular/core';

@Component({
  selector: 'app-mixed-queries',
  standalone: true,
  template: '<div #viewRef></div><ng-content></ng-content>',
})
export class MixedQueryComponent {
  @ViewChild('viewRef') viewRef!: ElementRef;
  @ViewChild('secondView') secondView!: ElementRef;
  @ContentChild('contentRef') contentRef!: TemplateRef<any>;
  @ContentChild('secondContent') secondContent!: TemplateRef<any>;
}
`.trim(),
    expectedFeatures: ['ɵɵviewQuery', 'ɵɵcontentQuery', 'ɵɵqueryRefresh'],
  },
]
