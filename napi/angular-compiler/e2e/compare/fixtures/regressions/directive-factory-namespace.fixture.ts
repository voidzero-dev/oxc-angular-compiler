/**
 * Regression: Directive factory DI tokens must be namespace-prefixed.
 *
 * ## The Issue
 *
 * `@Directive` classes that inject services from external modules emitted bare
 * identifiers like `ɵɵdirectiveInject(Store)` instead of namespace-prefixed
 * `ɵɵdirectiveInject(i1.Store)`. At runtime TypeScript elides the bare import
 * (it's type-only), causing `ASSERTION ERROR: token must be defined`.
 *
 * ## Root Cause (two parts)
 *
 * 1. `extract_param_token()` in directive/decorator.rs returned `ReadProp(i0.TypeName)`
 *    with a hardcoded `i0` prefix, instead of `ReadVar(TypeName)` like injectable/pipe.
 * 2. `transform_angular_file()` did not call `resolve_factory_dep_namespaces()` for
 *    directive deps.
 *
 * ## Fix
 *
 * Return `ReadVar(TypeName)` from `extract_param_token()` AND call
 * `resolve_factory_dep_namespaces()` for directive deps in transform.rs.
 *
 * ## Impact
 *
 * RUNTIME_AFFECTING: Without the fix, any directive with constructor DI from external
 * modules would crash at bootstrap with `ASSERTION ERROR: token must be defined`.
 * Discovered in ClickUp's ToastPositionHelperDirective which injects Store from @ngrx/store.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    type: 'full-transform',
    name: 'directive-factory-namespace-basic',
    category: 'regressions',
    description: 'Directive with external DI deps should use namespace-prefixed tokens',
    className: 'ToastPositionHelperDirective',
    sourceCode: `
import { Directive, ElementRef } from '@angular/core';
import { HttpClient } from '@angular/common/http';

@Directive({
  selector: '[appToastPosition]',
  standalone: true,
})
export class ToastPositionHelperDirective {
  constructor(
    private el: ElementRef,
    private http: HttpClient,
  ) {}
}
`.trim(),
    expectedFeatures: ['ɵɵdirectiveInject', 'ɵɵdefineDirective'],
  },
  {
    type: 'full-transform',
    name: 'directive-factory-namespace-multi-module',
    category: 'regressions',
    description:
      'Directive injecting from multiple external modules gets correct namespace indices',
    className: 'MultiDepDirective',
    sourceCode: `
import { Directive, ElementRef } from '@angular/core';
import { Router } from '@angular/router';
import { HttpClient } from '@angular/common/http';

@Directive({
  selector: '[appMultiDep]',
  standalone: true,
})
export class MultiDepDirective {
  constructor(
    private el: ElementRef,
    private router: Router,
    private http: HttpClient,
  ) {}
}
`.trim(),
    expectedFeatures: ['ɵɵdirectiveInject', 'ɵɵdefineDirective'],
  },
]
