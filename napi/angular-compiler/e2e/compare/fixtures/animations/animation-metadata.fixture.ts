/**
 * Animation metadata fixtures.
 *
 * Tests the animations property in @Component decorator which defines
 * Angular animation triggers, states, and transitions.
 *
 * The animations array contains animation trigger definitions using:
 * - trigger(): Defines an animation trigger with states and transitions
 * - state(): Defines a named animation state with styles
 * - transition(): Defines state-to-state animation transitions
 * - animate(): Specifies animation timing and easing
 * - style(): Defines CSS styles for animation states
 * - keyframes(): Defines multi-step animations
 * - group(): Groups animations to run in parallel
 * - sequence(): Runs animations in sequence
 * - query(): Queries child elements for animation
 * - stagger(): Staggers animations for multiple elements
 *
 * NOTE: The animations expressions are passed through to the compiled output.
 * These fixtures test that the compiler correctly handles the animations metadata,
 * not the actual animation runtime behavior.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  // ==========================================================================
  // Basic Animation Triggers
  // ==========================================================================

  {
    name: 'animation-metadata-basic-trigger',
    category: 'animations',
    description: 'Component with basic animation trigger',
    className: 'BasicAnimationTriggerComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, state, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-basic-trigger',
  standalone: true,
  template: \`<div [@fade]="state">Animated content</div>\`,
  animations: [
    trigger('fade', [
      state('void', style({ opacity: 0 })),
      state('*', style({ opacity: 1 })),
      transition('void => *', animate('300ms ease-in')),
      transition('* => void', animate('300ms ease-out'))
    ])
  ],
})
export class BasicAnimationTriggerComponent {
  state = 'initial';
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty'],
  },

  {
    name: 'animation-metadata-multiple-triggers',
    category: 'animations',
    description: 'Component with multiple animation triggers',
    className: 'MultipleTriggersComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-multiple-triggers',
  standalone: true,
  template: \`
    <div [@fadeIn]="fadeState" [@slideIn]="slideState">
      Multiple animations
    </div>
  \`,
  animations: [
    trigger('fadeIn', [
      transition(':enter', [
        style({ opacity: 0 }),
        animate('200ms', style({ opacity: 1 }))
      ])
    ]),
    trigger('slideIn', [
      transition(':enter', [
        style({ transform: 'translateX(-100%)' }),
        animate('300ms ease-out', style({ transform: 'translateX(0)' }))
      ])
    ])
  ],
})
export class MultipleTriggersComponent {
  fadeState = 'initial';
  slideState = 'initial';
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty'],
  },

  // ==========================================================================
  // State Definitions
  // ==========================================================================

  {
    name: 'animation-metadata-named-states',
    category: 'animations',
    description: 'Animation with named states',
    className: 'NamedStatesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, state, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-named-states',
  standalone: true,
  template: \`<div [@toggle]="isOpen ? 'open' : 'closed'">Toggle me</div>\`,
  animations: [
    trigger('toggle', [
      state('closed', style({
        height: '0px',
        opacity: 0,
        overflow: 'hidden'
      })),
      state('open', style({
        height: '*',
        opacity: 1
      })),
      transition('closed <=> open', animate('250ms ease-in-out'))
    ])
  ],
})
export class NamedStatesComponent {
  isOpen = false;
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty'],
  },

  {
    name: 'animation-metadata-wildcard-state',
    category: 'animations',
    description: 'Animation with wildcard state',
    className: 'WildcardStateComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, state, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-wildcard-state',
  standalone: true,
  template: \`<div [@colorChange]="currentColor">Color changing</div>\`,
  animations: [
    trigger('colorChange', [
      state('red', style({ backgroundColor: 'red' })),
      state('green', style({ backgroundColor: 'green' })),
      state('blue', style({ backgroundColor: 'blue' })),
      transition('* => *', animate('300ms ease'))
    ])
  ],
})
export class WildcardStateComponent {
  currentColor = 'red';
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty'],
  },

  // ==========================================================================
  // Transitions
  // ==========================================================================

  {
    name: 'animation-metadata-enter-leave',
    category: 'animations',
    description: 'Animation with :enter and :leave aliases',
    className: 'EnterLeaveComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-enter-leave',
  standalone: true,
  template: \`
    @if (visible) {
      <div [@enterLeave]>Enter/Leave animation</div>
    }
  \`,
  animations: [
    trigger('enterLeave', [
      transition(':enter', [
        style({ opacity: 0, transform: 'scale(0.8)' }),
        animate('200ms ease-out', style({ opacity: 1, transform: 'scale(1)' }))
      ]),
      transition(':leave', [
        animate('150ms ease-in', style({ opacity: 0, transform: 'scale(0.8)' }))
      ])
    ])
  ],
})
export class EnterLeaveComponent {
  visible = false;
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty', 'ɵɵconditional'],
  },

  {
    name: 'animation-metadata-increment-decrement',
    category: 'animations',
    description: 'Animation with :increment and :decrement transitions',
    className: 'IncrementDecrementComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-increment-decrement',
  standalone: true,
  template: \`<div [@counter]="count">{{ count }}</div>\`,
  animations: [
    trigger('counter', [
      transition(':increment', [
        style({ color: 'green', transform: 'translateY(-10px)' }),
        animate('200ms', style({ color: '*', transform: 'translateY(0)' }))
      ]),
      transition(':decrement', [
        style({ color: 'red', transform: 'translateY(10px)' }),
        animate('200ms', style({ color: '*', transform: 'translateY(0)' }))
      ])
    ])
  ],
})
export class IncrementDecrementComponent {
  count = 0;
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty'],
  },

  {
    name: 'animation-metadata-bidirectional',
    category: 'animations',
    description: 'Bidirectional transition with <=>',
    className: 'BidirectionalTransitionComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, state, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-bidirectional',
  standalone: true,
  template: \`<div [@expand]="isExpanded">Expandable</div>\`,
  animations: [
    trigger('expand', [
      state('false', style({ height: '100px' })),
      state('true', style({ height: '300px' })),
      transition('false <=> true', animate('250ms cubic-bezier(0.4, 0, 0.2, 1)'))
    ])
  ],
})
export class BidirectionalTransitionComponent {
  isExpanded = false;
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty'],
  },

  // ==========================================================================
  // Advanced Animation Features
  // ==========================================================================

  {
    name: 'animation-metadata-keyframes',
    category: 'animations',
    description: 'Animation with keyframes',
    className: 'KeyframesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, animate, style, keyframes } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-keyframes',
  standalone: true,
  template: \`<div [@bounce]="trigger">Bouncing</div>\`,
  animations: [
    trigger('bounce', [
      transition('* => *', [
        animate('500ms ease-in-out', keyframes([
          style({ transform: 'translateY(0)', offset: 0 }),
          style({ transform: 'translateY(-20px)', offset: 0.3 }),
          style({ transform: 'translateY(0)', offset: 0.5 }),
          style({ transform: 'translateY(-10px)', offset: 0.7 }),
          style({ transform: 'translateY(0)', offset: 1 })
        ]))
      ])
    ])
  ],
})
export class KeyframesComponent {
  trigger = 'initial';
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty'],
  },

  {
    name: 'animation-metadata-group',
    category: 'animations',
    description: 'Parallel animations with group()',
    className: 'GroupAnimationComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, animate, style, group } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-group',
  standalone: true,
  template: \`<div [@parallel]="state">Parallel animations</div>\`,
  animations: [
    trigger('parallel', [
      transition(':enter', [
        style({ opacity: 0, transform: 'translateX(-100%)' }),
        group([
          animate('300ms ease', style({ opacity: 1 })),
          animate('400ms ease-out', style({ transform: 'translateX(0)' }))
        ])
      ])
    ])
  ],
})
export class GroupAnimationComponent {
  state = 'initial';
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty'],
  },

  {
    name: 'animation-metadata-sequence',
    category: 'animations',
    description: 'Sequential animations with sequence()',
    className: 'SequenceAnimationComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, animate, style, sequence } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-sequence',
  standalone: true,
  template: \`<div [@sequential]="state">Sequential animations</div>\`,
  animations: [
    trigger('sequential', [
      transition(':enter', [
        sequence([
          style({ opacity: 0, transform: 'scale(0.5)' }),
          animate('200ms', style({ opacity: 1 })),
          animate('200ms', style({ transform: 'scale(1)' }))
        ])
      ])
    ])
  ],
})
export class SequenceAnimationComponent {
  state = 'initial';
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty'],
  },

  {
    name: 'animation-metadata-query',
    category: 'animations',
    description: 'Child element animations with query()',
    className: 'QueryAnimationComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, animate, style, query, stagger } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-query',
  standalone: true,
  template: \`
    <div [@listAnimation]="items.length">
      @for (item of items; track item.id) {
        <div class="item">{{ item.name }}</div>
      }
    </div>
  \`,
  animations: [
    trigger('listAnimation', [
      transition('* => *', [
        query(':enter', [
          style({ opacity: 0, transform: 'translateY(-20px)' }),
          stagger(50, [
            animate('200ms ease-out', style({ opacity: 1, transform: 'translateY(0)' }))
          ])
        ], { optional: true }),
        query(':leave', [
          stagger(50, [
            animate('200ms ease-in', style({ opacity: 0, transform: 'translateY(20px)' }))
          ])
        ], { optional: true })
      ])
    ])
  ],
})
export class QueryAnimationComponent {
  items: { id: number; name: string }[] = [];
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty', 'ɵɵrepeaterCreate'],
  },

  {
    name: 'animation-metadata-stagger',
    category: 'animations',
    description: 'Staggered animations for lists',
    className: 'StaggerAnimationComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, animate, style, query, stagger } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-stagger',
  standalone: true,
  template: \`
    <ul [@staggerList]="items.length">
      @for (item of items; track item.id) {
        <li>{{ item.name }}</li>
      }
    </ul>
  \`,
  animations: [
    trigger('staggerList', [
      transition('* => *', [
        query(':enter', [
          style({ opacity: 0 }),
          stagger('100ms', [
            animate('300ms ease-out', style({ opacity: 1 }))
          ])
        ], { optional: true })
      ])
    ])
  ],
})
export class StaggerAnimationComponent {
  items: { id: number; name: string }[] = [];
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty', 'ɵɵrepeaterCreate'],
  },

  // ==========================================================================
  // Animation with Component Metadata
  // ==========================================================================

  {
    name: 'animation-metadata-with-host-bindings',
    category: 'animations',
    description: 'Animations combined with host bindings',
    className: 'AnimationsWithHostBindingsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, state, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-with-host-bindings',
  standalone: true,
  template: \`<div>Content</div>\`,
  animations: [
    trigger('hostFade', [
      state('visible', style({ opacity: 1 })),
      state('hidden', style({ opacity: 0 })),
      transition('* <=> *', animate('200ms'))
    ])
  ],
  host: {
    '[@hostFade]': 'visibility',
    '[class.animated]': 'isAnimated',
  },
})
export class AnimationsWithHostBindingsComponent {
  visibility = 'visible';
  isAnimated = true;
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  {
    name: 'animation-metadata-with-encapsulation',
    category: 'animations',
    description: 'Animations with ViewEncapsulation.None',
    className: 'AnimationsWithEncapsulationComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';
import { trigger, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-with-encapsulation',
  standalone: true,
  template: \`<div [@globalFade]="state">Global styles animation</div>\`,
  animations: [
    trigger('globalFade', [
      transition(':enter', [
        style({ opacity: 0 }),
        animate('300ms', style({ opacity: 1 }))
      ])
    ])
  ],
  encapsulation: ViewEncapsulation.None,
})
export class AnimationsWithEncapsulationComponent {
  state = 'initial';
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty', 'encapsulation: 2'],
  },

  {
    name: 'animation-metadata-with-change-detection',
    ngtscMode: 'full',
    category: 'animations',
    description: 'Animations with OnPush change detection',
    className: 'AnimationsWithOnPushComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ChangeDetectionStrategy } from '@angular/core';
import { trigger, transition, animate } from '@angular/animations';
import { AsyncPipe } from '@angular/common';
import { Observable, of } from 'rxjs';

@Component({
  selector: 'app-animation-metadata-with-change-detection',
  standalone: true,
  imports: [AsyncPipe],
  template: \`<div [@asyncFade]="state$ | async">Async state</div>\`,
  animations: [
    trigger('asyncFade', [
      transition('* => *', animate('200ms'))
    ])
  ],
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class AnimationsWithOnPushComponent {
  state$: Observable<string> = of('initial');
}
    `.trim(),
    expectedFeatures: [
      'ɵɵdefineComponent',
      'ɵɵsyntheticHostProperty',
      'ChangeDetectionStrategy.OnPush',
      'ɵɵpipe',
    ],
  },

  // ==========================================================================
  // Real-World Animation Scenarios
  // ==========================================================================

  {
    name: 'animation-metadata-modal',
    category: 'animations',
    description: 'Modal dialog animations',
    className: 'AnimatedModalComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-modal',
  standalone: true,
  template: \`
    @if (isOpen) {
      <div class="overlay" [@overlayFade]>
        <div class="modal" [@modalSlide]>
          <header>{{ title }}</header>
          <main><ng-content></ng-content></main>
          <footer>
            <button (click)="close()">Close</button>
          </footer>
        </div>
      </div>
    }
  \`,
  animations: [
    trigger('overlayFade', [
      transition(':enter', [
        style({ opacity: 0 }),
        animate('150ms ease-out', style({ opacity: 1 }))
      ]),
      transition(':leave', [
        animate('150ms ease-in', style({ opacity: 0 }))
      ])
    ]),
    trigger('modalSlide', [
      transition(':enter', [
        style({ opacity: 0, transform: 'translateY(-50px) scale(0.95)' }),
        animate('200ms 50ms ease-out', style({ opacity: 1, transform: 'translateY(0) scale(1)' }))
      ]),
      transition(':leave', [
        animate('150ms ease-in', style({ opacity: 0, transform: 'translateY(-30px) scale(0.95)' }))
      ])
    ])
  ],
})
export class AnimatedModalComponent {
  isOpen = false;
  title = '';

  close() {}
}
    `.trim(),
    expectedFeatures: [
      'ɵɵdefineComponent',
      'ɵɵsyntheticHostProperty',
      'ɵɵconditional',
      'ɵɵprojection',
      'ɵɵlistener',
    ],
  },

  {
    name: 'animation-metadata-accordion',
    category: 'animations',
    description: 'Accordion panel animations',
    className: 'AnimatedAccordionComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-accordion',
  standalone: true,
  template: \`
    @for (panel of panels; track panel.id) {
      <div class="panel">
        <button (click)="toggle(panel.id)">
          {{ panel.title }}
        </button>
        @if (expandedId === panel.id) {
          <div class="content" [@expand]>
            {{ panel.content }}
          </div>
        }
      </div>
    }
  \`,
  animations: [
    trigger('expand', [
      transition(':enter', [
        style({ height: '0', opacity: 0, overflow: 'hidden' }),
        animate('250ms ease-out', style({ height: '*', opacity: 1 }))
      ]),
      transition(':leave', [
        style({ overflow: 'hidden' }),
        animate('200ms ease-in', style({ height: '0', opacity: 0 }))
      ])
    ])
  ],
})
export class AnimatedAccordionComponent {
  panels: { id: number; title: string; content: string }[] = [];
  expandedId: number | null = null;

  toggle(id: number) {}
}
    `.trim(),
    expectedFeatures: [
      'ɵɵdefineComponent',
      'ɵɵsyntheticHostProperty',
      'ɵɵrepeaterCreate',
      'ɵɵconditional',
      'ɵɵlistener',
    ],
  },

  {
    name: 'animation-metadata-router-transition',
    category: 'animations',
    description: 'Router outlet transition animations',
    className: 'RouterTransitionComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { RouterOutlet } from '@angular/router';
import { trigger, transition, animate, style, query, group } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-router-transition',
  standalone: true,
  imports: [RouterOutlet],
  template: \`
    <main [@routeAnimation]="outlet.activatedRouteData?.['animation']">
      <router-outlet #outlet="outlet"></router-outlet>
    </main>
  \`,
  animations: [
    trigger('routeAnimation', [
      transition('* <=> *', [
        style({ position: 'relative' }),
        query(':enter, :leave', [
          style({
            position: 'absolute',
            top: 0,
            left: 0,
            width: '100%'
          })
        ], { optional: true }),
        query(':enter', [style({ opacity: 0 })], { optional: true }),
        group([
          query(':leave', [
            animate('200ms ease-in', style({ opacity: 0 }))
          ], { optional: true }),
          query(':enter', [
            animate('300ms 100ms ease-out', style({ opacity: 1 }))
          ], { optional: true })
        ])
      ])
    ])
  ],
})
export class RouterTransitionComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty'],
  },

  {
    name: 'animation-metadata-notification',
    category: 'animations',
    description: 'Toast/notification animations',
    className: 'NotificationComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-notification',
  standalone: true,
  template: \`
    @for (notification of notifications; track notification.id) {
      <div class="notification" [@notificationAnim]="notification.type"
           [class]="notification.type">
        <span>{{ notification.message }}</span>
        <button (click)="dismiss(notification.id)">x</button>
      </div>
    }
  \`,
  animations: [
    trigger('notificationAnim', [
      transition(':enter', [
        style({ transform: 'translateX(100%)', opacity: 0 }),
        animate('300ms ease-out', style({ transform: 'translateX(0)', opacity: 1 }))
      ]),
      transition(':leave', [
        animate('200ms ease-in', style({ transform: 'translateX(100%)', opacity: 0 }))
      ])
    ])
  ],
})
export class NotificationComponent {
  notifications: { id: number; type: string; message: string }[] = [];

  dismiss(id: number) {}
}
    `.trim(),
    expectedFeatures: [
      'ɵɵdefineComponent',
      'ɵɵsyntheticHostProperty',
      'ɵɵrepeaterCreate',
      'ɵɵlistener',
    ],
  },

  // ==========================================================================
  // Edge Cases
  // ==========================================================================

  {
    name: 'animation-metadata-empty-trigger',
    category: 'animations',
    description: 'Animation trigger with no transitions',
    className: 'EmptyTriggerComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-empty-trigger',
  standalone: true,
  template: \`<div [@empty]="state">Empty trigger</div>\`,
  animations: [trigger('empty', [])],
})
export class EmptyTriggerComponent {
  state = 'initial';
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty'],
  },

  {
    name: 'animation-metadata-void-state',
    category: 'animations',
    description: 'Explicit void state handling',
    className: 'VoidStateComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, state, transition, animate, style } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-void-state',
  standalone: true,
  template: \`
    @if (show) {
      <div [@voidAnim]>Void animation</div>
    }
  \`,
  animations: [
    trigger('voidAnim', [
      state('void', style({ opacity: 0, height: 0 })),
      state('*', style({ opacity: 1, height: '*' })),
      transition('void <=> *', animate('200ms'))
    ])
  ],
})
export class VoidStateComponent {
  show = false;
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty', 'ɵɵconditional'],
  },

  {
    name: 'animation-metadata-disabled',
    category: 'animations',
    description: 'Animation with disabled binding',
    className: 'DisabledAnimationComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { trigger, transition, animate } from '@angular/animations';

@Component({
  selector: 'app-animation-metadata-disabled',
  standalone: true,
  template: \`<div [@.disabled]="disableAnimations" [@fade]="state">May be disabled</div>\`,
  animations: [
    trigger('fade', [
      transition('* => *', animate('200ms'))
    ])
  ],
})
export class DisabledAnimationComponent {
  disableAnimations = false;
  state = 'initial';
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵsyntheticHostProperty'],
  },
]
