import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'formfield-alias-repro',
    category: 'regressions',
    description: 'Issue #229 repro: [formField] binding with aliased field input',
    className: 'AppComponent',
    type: 'full-transform',
    sourceCode: `
import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { form, FormField, required } from '@angular/forms/signals';

@Component({
  selector: 'app-root',
  imports: [FormField],
  template: \`<input type="text" [formField]="myForm.firstName" />\`,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AppComponent {
  protected readonly myFormModel = signal({
    firstName: 'Foo',
    email: 'foo@bar.com',
  });
  protected readonly myForm = form(this.myFormModel, path => {
    required(path.firstName);
  });
}
`.trim(),
    expectedFeatures: ['ɵɵproperty', 'ɵɵcontrol', 'ɵɵcontrolCreate'],
  },
]
