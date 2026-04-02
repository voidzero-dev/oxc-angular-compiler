/**
 * Host animation bindings (component-level).
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'animation-host-property-trigger',
    category: 'animations',
    description: 'Animation trigger binding in host property',
    className: 'AnimationHostTriggerComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, style, animate } from '@angular/animations';

@Component({
  selector: 'app-animation-host-trigger',
  standalone: true,
  template: \`<ng-content></ng-content>\`,
  animations: [
    trigger('slideIn', [
      transition(':enter', [
        style({ width: 0, opacity: 0 }),
        animate('200ms ease-out', style({ width: '*', opacity: 1 })),
      ]),
      transition(':leave', [
        animate('200ms ease-in', style({ width: 0, opacity: 0 })),
      ]),
    ]),
  ],
  host: {
    '[@slideIn]': 'animationState',
  }
})
export class AnimationHostTriggerComponent {
  animationState = 'active';
}
    `.trim(),
    expectedFeatures: ['ɵɵsyntheticHostProperty'],
  },
  {
    name: 'animation-host-property-trigger-with-style',
    category: 'animations',
    description: 'Animation trigger binding combined with style binding in host property',
    className: 'AnimationHostTriggerWithStyleComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, style, animate } from '@angular/animations';

@Component({
  selector: 'app-animation-host-trigger-with-style',
  standalone: true,
  template: \`<ng-content></ng-content>\`,
  animations: [
    trigger('slideIn', [
      transition(':enter', [
        style({ opacity: 0 }),
        animate('200ms ease-out', style({ opacity: 1 })),
      ]),
    ]),
  ],
  host: {
    '[@slideIn]': 'animationState',
    '[style.overflow]': '"hidden"',
  }
})
export class AnimationHostTriggerWithStyleComponent {
  animationState = 'active';
}
    `.trim(),
    expectedFeatures: ['ɵɵsyntheticHostProperty', 'ɵɵstyleProp'],
  },
  {
    name: 'animation-on-component',
    category: 'animations',
    description: 'Animation on child component',
    className: 'AnimationOnComponentComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-animation-on-component',
  standalone: true,
  template: \`
    <app-child [@componentAnim]="state"></app-child>
  \`,
})
export class AnimationOnComponentComponent {
  state = 'initial';
}
    `.trim(),
    expectedFeatures: ['ɵɵsyntheticHostProperty'],
  },
  {
    name: 'animation-with-property',
    category: 'animations',
    description: 'Animation combined with property binding',
    className: 'AnimationWithPropertyComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-animation-with-property',
  standalone: true,
  template: \`
    <div [@highlight]="isHighlighted" [class.active]="isActive">Combined</div>
  \`,
})
export class AnimationWithPropertyComponent {
  isHighlighted = false;
  isActive = false;
}
    `.trim(),
    expectedFeatures: ['ɵɵsyntheticHostProperty', 'ɵɵclassProp'],
  },
  {
    name: 'animation-params',
    category: 'animations',
    description: 'Animation with params object',
    className: 'AnimationParamsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-animation-params',
  standalone: true,
  template: \`
    <div [@fade]="{ value: state, params: { duration: 500 } }">Parameterized</div>
  \`,
})
export class AnimationParamsComponent {
  state = 'initial';
}
    `.trim(),
    expectedFeatures: ['ɵɵsyntheticHostProperty'],
  },
  {
    name: 'animation-directive-host-property-trigger',
    category: 'animations',
    description: 'Animation trigger binding in directive host property',
    className: 'SlideDirective',
    type: 'full-transform',
    sourceCode: `
import { Directive } from '@angular/core';
import { trigger, transition, style, animate } from '@angular/animations';

@Directive({
  selector: '[appSlide]',
  host: {
    '[@slideIn]': 'animationState',
  }
})
export class SlideDirective {
  animationState = 'active';
}
    `.trim(),
    expectedFeatures: ['ɵɵsyntheticHostProperty'],
  },
]
