/**
 * `@Inject('TOKEN')` with a string token on a component constructor.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'component-inject-string-token',
    category: 'factory',
    description: 'component factory injects string tokens, with flags',
    className: 'InjectStringTokenComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, Inject, Optional, Self } from '@angular/core';

@Component({
  selector: 'app-inject-string-token',
  template: '',
})
export class InjectStringTokenComponent {
  constructor(
    @Inject('API_URL') url: string,
    @Optional() @Self() @Inject('FLAGS') flags: string[],
    @Inject(('PAREN')) paren: string,
  ) {}
}
    `.trim(),
    expectedFeatures: ['ɵɵdirectiveInject'],
  },
]
