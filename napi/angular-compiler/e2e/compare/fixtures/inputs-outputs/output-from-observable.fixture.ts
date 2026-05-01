/**
 * Fixtures for outputFromObservable() from @angular/core/rxjs-interop.
 *
 * outputFromObservable() is equivalent to output() for metadata purposes —
 * both register an output binding — but it wraps an existing RxJS observable
 * instead of creating a new OutputEmitterRef. Options (e.g. alias) are passed
 * as the *second* argument, unlike output() where they are the first.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    type: 'full-transform',
    name: 'output-from-observable-simple',
    category: 'inputs-outputs',
    description: 'outputFromObservable with a simple EventEmitter should appear in outputs metadata',
    className: 'OutputFromObservableSimpleComponent',
    sourceCode: `
import { Component, EventEmitter } from '@angular/core';
import { outputFromObservable } from '@angular/core/rxjs-interop';

@Component({
  selector: 'app-output-from-observable-simple',
  standalone: true,
  template: '',
})
export class OutputFromObservableSimpleComponent {
  readonly queryChanged = outputFromObservable(new EventEmitter<string>());
  readonly clicked = outputFromObservable(new EventEmitter<void>());
}
`.trim(),
  },
  {
    type: 'full-transform',
    name: 'output-from-observable-piped',
    category: 'inputs-outputs',
    description: 'outputFromObservable with a piped observable chain should appear in outputs metadata',
    className: 'OutputFromObservablePipedComponent',
    sourceCode: `
import { Component } from '@angular/core';
import { outputFromObservable } from '@angular/core/rxjs-interop';
import { Subject } from 'rxjs';
import { skip, debounceTime } from 'rxjs/operators';

@Component({
  selector: 'app-output-from-observable-piped',
  standalone: true,
  template: '',
})
export class OutputFromObservablePipedComponent {
  private query$ = new Subject<string>();

  readonly queryChanged = outputFromObservable(
    this.query$.pipe(
      skip(1),
      debounceTime(300),
    ),
  );
}
`.trim(),
  },
  {
    type: 'full-transform',
    name: 'output-from-observable-alias',
    category: 'inputs-outputs',
    description: 'outputFromObservable with alias in second argument should use alias as binding name',
    className: 'OutputFromObservableAliasComponent',
    sourceCode: `
import { Component, EventEmitter } from '@angular/core';
import { outputFromObservable } from '@angular/core/rxjs-interop';

@Component({
  selector: 'app-output-from-observable-alias',
  standalone: true,
  template: '',
})
export class OutputFromObservableAliasComponent {
  readonly _clicked = outputFromObservable(new EventEmitter<void>(), { alias: 'clicked' });
}
`.trim(),
  },
  {
    type: 'full-transform',
    name: 'output-from-observable-mixed',
    category: 'inputs-outputs',
    description: 'Class using both output() and outputFromObservable() should have both in outputs',
    className: 'OutputFromObservableMixedComponent',
    sourceCode: `
import { Component, EventEmitter, output } from '@angular/core';
import { outputFromObservable } from '@angular/core/rxjs-interop';

@Component({
  selector: 'app-output-from-observable-mixed',
  standalone: true,
  template: '',
})
export class OutputFromObservableMixedComponent {
  readonly clicked = output<void>();
  readonly queryChanged = outputFromObservable(new EventEmitter<string>());
}
`.trim(),
  },
]
