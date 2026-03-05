/**
 * Host directive fixtures.
 *
 * Tests the hostDirectives option in @Component/@Directive decorator:
 * - Basic host directive application
 * - Input/output mappings (aliasing)
 * - Multiple host directives
 * - Host directives with DI
 *
 * Host directives allow composition of directive behaviors without inheritance.
 * They are applied to the host element automatically when the component is used.
 *
 * All fixtures use full-transform type to provide complete TypeScript source
 * with proper directive declarations for NgtscProgram compilation.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  // ==========================================================================
  // Basic Host Directives
  // ==========================================================================

  {
    type: 'full-transform',
    name: 'host-directive-basic',
    category: 'host-directives',
    description: 'Component with single host directive',
    className: 'BasicHostDirectiveComponent',
    sourceCode: `
import { Component, Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[tooltip]', standalone: true })
export class TooltipDirective {
  @Input() tooltipText: string = '';
}

@Component({
  selector: 'app-basic-host-directive',
  standalone: true,
  hostDirectives: [TooltipDirective],
  template: \`<div>Host directive applied</div>\`
})
export class BasicHostDirectiveComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  {
    type: 'full-transform',
    name: 'host-directive-multiple',
    category: 'host-directives',
    description: 'Component with multiple host directives',
    className: 'MultipleHostDirectivesComponent',
    sourceCode: `
import { Component, Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[tooltip]', standalone: true })
export class TooltipDirective {}

@Directive({ selector: '[highlight]', standalone: true })
export class HighlightDirective {}

@Directive({ selector: '[dragdrop]', standalone: true })
export class DragDropDirective {}

@Component({
  selector: 'app-multiple-host-directives',
  standalone: true,
  hostDirectives: [TooltipDirective, HighlightDirective, DragDropDirective],
  template: \`<div>Multiple directives</div>\`
})
export class MultipleHostDirectivesComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  // ==========================================================================
  // Host Directives with Input Mappings
  // ==========================================================================

  {
    type: 'full-transform',
    name: 'host-directive-input-mapping',
    category: 'host-directives',
    description: 'Host directive with input mappings',
    className: 'InputMappingHostDirectiveComponent',
    sourceCode: `
import { Component, Directive, Input } from '@angular/core';

@Directive({ selector: '[color]', standalone: true })
export class ColorDirective {
  @Input() bgColor: string = '';
}

@Component({
  selector: 'app-input-mapping',
  standalone: true,
  hostDirectives: [{
    directive: ColorDirective,
    inputs: ['bgColor: color']
  }],
  template: \`<div>Input mapped</div>\`
})
export class InputMappingHostDirectiveComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  {
    type: 'full-transform',
    name: 'host-directive-multiple-input-mappings',
    category: 'host-directives',
    description: 'Host directive with multiple input mappings',
    className: 'MultipleInputMappingsComponent',
    sourceCode: `
import { Component, Directive, Input } from '@angular/core';

@Directive({ selector: '[size]', standalone: true })
export class SizeDirective {
  @Input() sizeWidth: number = 0;
  @Input() sizeHeight: number = 0;
  @Input() sizeUnit: string = 'px';
}

@Component({
  selector: 'app-multiple-inputs',
  standalone: true,
  hostDirectives: [{
    directive: SizeDirective,
    inputs: ['sizeWidth: width', 'sizeHeight: height', 'sizeUnit: unit']
  }],
  template: \`<div>Multiple inputs mapped</div>\`
})
export class MultipleInputMappingsComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  {
    type: 'full-transform',
    name: 'host-directive-same-name-input',
    category: 'host-directives',
    description: 'Host directive input with same public and internal name',
    className: 'SameNameInputComponent',
    sourceCode: `
import { Component, Directive, Input } from '@angular/core';

@Directive({ selector: '[opacity]', standalone: true })
export class OpacityDirective {
  @Input() opacity: number = 1;
}

@Component({
  selector: 'app-same-name',
  standalone: true,
  hostDirectives: [{
    directive: OpacityDirective,
    inputs: ['opacity']
  }],
  template: \`<div>Same name input</div>\`
})
export class SameNameInputComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  // ==========================================================================
  // Host Directives with Output Mappings
  // ==========================================================================

  {
    type: 'full-transform',
    name: 'host-directive-output-mapping',
    category: 'host-directives',
    description: 'Host directive with output mappings',
    className: 'OutputMappingHostDirectiveComponent',
    sourceCode: `
import { Component, Directive, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[clickTracking]', standalone: true })
export class ClickTrackingDirective {
  @Output() trackClick = new EventEmitter<void>();
}

@Component({
  selector: 'app-output-mapping',
  standalone: true,
  hostDirectives: [{
    directive: ClickTrackingDirective,
    outputs: ['trackClick: clicked']
  }],
  template: \`<div>Output mapped</div>\`
})
export class OutputMappingHostDirectiveComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  {
    type: 'full-transform',
    name: 'host-directive-multiple-output-mappings',
    category: 'host-directives',
    description: 'Host directive with multiple output mappings',
    className: 'MultipleOutputMappingsComponent',
    sourceCode: `
import { Component, Directive, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[gesture]', standalone: true })
export class GestureDirective {
  @Output() onSwipeLeft = new EventEmitter<void>();
  @Output() onSwipeRight = new EventEmitter<void>();
  @Output() onTap = new EventEmitter<void>();
  @Output() onLongPress = new EventEmitter<void>();
}

@Component({
  selector: 'app-multiple-outputs',
  standalone: true,
  hostDirectives: [{
    directive: GestureDirective,
    outputs: ['onSwipeLeft: swipeLeft', 'onSwipeRight: swipeRight', 'onTap: tap', 'onLongPress: longPress']
  }],
  template: \`<div>Multiple outputs</div>\`
})
export class MultipleOutputMappingsComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  // ==========================================================================
  // Host Directives with Both Input and Output Mappings
  // ==========================================================================

  {
    type: 'full-transform',
    name: 'host-directive-input-output-mapping',
    category: 'host-directives',
    description: 'Host directive with both input and output mappings',
    className: 'InputOutputMappingComponent',
    sourceCode: `
import { Component, Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[resizable]', standalone: true })
export class ResizableDirective {
  @Input() resizeMinWidth: number = 0;
  @Input() resizeMaxWidth: number = 1000;
  @Output() onResized = new EventEmitter<{width: number, height: number}>();
}

@Component({
  selector: 'app-input-output-mapping',
  standalone: true,
  hostDirectives: [{
    directive: ResizableDirective,
    inputs: ['resizeMinWidth: minWidth', 'resizeMaxWidth: maxWidth'],
    outputs: ['onResized: resized']
  }],
  template: \`<div>Input and output mapped</div>\`
})
export class InputOutputMappingComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  {
    type: 'full-transform',
    name: 'host-directive-complex-mappings',
    category: 'host-directives',
    description: 'Host directive with complex input/output configuration',
    className: 'ComplexMappingsComponent',
    sourceCode: `
import { Component, Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[draggable]', standalone: true })
export class DraggableDirective {
  @Input() draggable: boolean = true;
  @Input() axis: 'x' | 'y' | 'both' = 'both';
  @Input() bounds: string = '';
  @Input() handle: string = '';
  @Output() onDragStart = new EventEmitter<void>();
  @Output() onDragMove = new EventEmitter<{x: number, y: number}>();
  @Output() onDragEnd = new EventEmitter<void>();
}

@Component({
  selector: 'app-complex-mappings',
  standalone: true,
  hostDirectives: [{
    directive: DraggableDirective,
    inputs: ['draggable: dragEnabled', 'axis: dragAxis', 'bounds: dragBounds', 'handle: dragHandle'],
    outputs: ['onDragStart: dragStart', 'onDragMove: dragMove', 'onDragEnd: dragEnd']
  }],
  template: \`<div>Complex mappings</div>\`
})
export class ComplexMappingsComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  // ==========================================================================
  // Multiple Host Directives with Mappings
  // ==========================================================================

  {
    type: 'full-transform',
    name: 'host-directives-multiple-with-mappings',
    category: 'host-directives',
    description: 'Multiple host directives each with their own mappings',
    className: 'MultipleDirectivesMappingsComponent',
    sourceCode: `
import { Component, Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[tooltip]', standalone: true })
export class TooltipDirective {
  @Input() tooltipText: string = '';
}

@Directive({ selector: '[highlight]', standalone: true })
export class HighlightDirective {
  @Input() color: string = '';
  @Output() onHighlight = new EventEmitter<void>();
}

@Component({
  selector: 'app-multiple-with-mappings',
  standalone: true,
  hostDirectives: [
    { directive: TooltipDirective, inputs: ['tooltipText: tooltip'] },
    { directive: HighlightDirective, inputs: ['color: highlightColor'], outputs: ['onHighlight: highlighted'] }
  ],
  template: \`<div>Multiple with mappings</div>\`
})
export class MultipleDirectivesMappingsComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  {
    type: 'full-transform',
    name: 'host-directives-mixed',
    category: 'host-directives',
    description: 'Mix of host directives with and without mappings',
    className: 'MixedHostDirectivesComponent',
    sourceCode: `
import { Component, Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[accessibility]', standalone: true })
export class AccessibilityDirective {}

@Directive({ selector: '[animation]', standalone: true })
export class AnimationDirective {
  @Input() animationType: string = '';
}

@Directive({ selector: '[focus]', standalone: true })
export class FocusDirective {}

@Directive({ selector: '[theme]', standalone: true })
export class ThemeDirective {
  @Input() themeName: string = '';
  @Output() onThemeChange = new EventEmitter<string>();
}

@Component({
  selector: 'app-mixed-directives',
  standalone: true,
  hostDirectives: [
    AccessibilityDirective,
    { directive: AnimationDirective, inputs: ['animationType: animation'] },
    FocusDirective,
    { directive: ThemeDirective, inputs: ['themeName: theme'], outputs: ['onThemeChange: themeChanged'] }
  ],
  template: \`<div>Mixed directives</div>\`
})
export class MixedHostDirectivesComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  // ==========================================================================
  // Host Directives with Component Metadata
  // ==========================================================================

  {
    type: 'full-transform',
    name: 'host-directives-with-host-bindings',
    category: 'host-directives',
    description: 'Host directives combined with host property bindings',
    className: 'HostDirectivesWithHostBindingsComponent',
    sourceCode: `
import { Component, Directive } from '@angular/core';

@Directive({ selector: '[tooltip]', standalone: true })
export class TooltipDirective {}

@Component({
  selector: 'app-host-bindings',
  standalone: true,
  hostDirectives: [TooltipDirective],
  host: {
    '[class.active]': 'isActive',
    '[style.cursor]': 'cursorStyle'
  },
  template: \`<div>Host bindings + directives</div>\`
})
export class HostDirectivesWithHostBindingsComponent {
  isActive = false;
  cursorStyle = 'pointer';
}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  {
    type: 'full-transform',
    name: 'host-directives-with-providers',
    category: 'host-directives',
    description: 'Host directives combined with providers',
    className: 'HostDirectivesWithProvidersComponent',
    sourceCode: `
import { Component, Directive, Input, Injectable } from '@angular/core';

@Injectable()
export class DataService {
  data = 'test data';
}

@Directive({ selector: '[dataSource]', standalone: true })
export class DataSourceDirective {
  @Input() source: string = '';
}

@Component({
  selector: 'app-with-providers',
  standalone: true,
  hostDirectives: [{ directive: DataSourceDirective, inputs: ['source: dataSource'] }],
  providers: [DataService],
  template: \`<div>{{ service.data }}</div>\`
})
export class HostDirectivesWithProvidersComponent {
  constructor(public service: DataService) {}
}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  {
    type: 'full-transform',
    name: 'host-directives-with-encapsulation',
    category: 'host-directives',
    description: 'Host directives with ViewEncapsulation.None',
    className: 'HostDirectivesEncapsulationComponent',
    sourceCode: `
import { Component, Directive, ViewEncapsulation } from '@angular/core';

@Directive({ selector: '[style]', standalone: true })
export class StyleDirective {}

@Component({
  selector: 'app-encapsulation',
  standalone: true,
  hostDirectives: [StyleDirective],
  encapsulation: ViewEncapsulation.None,
  template: \`<div class="global">Styled</div>\`
})
export class HostDirectivesEncapsulationComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature', 'encapsulation: 2'],
  },

  {
    type: 'full-transform',
    name: 'host-directives-with-change-detection',
    category: 'host-directives',
    description: 'Host directives with OnPush change detection',
    className: 'HostDirectivesOnPushComponent',
    sourceCode: `
import { Component, Directive, ChangeDetectionStrategy } from '@angular/core';
import { AsyncPipe } from '@angular/common';

@Directive({ selector: '[async]', standalone: true })
export class AsyncDirective {}

@Component({
  selector: 'app-onpush',
  standalone: true,
  imports: [AsyncPipe],
  hostDirectives: [AsyncDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: \`<div>{{ data$ | async }}</div>\`
})
export class HostDirectivesOnPushComponent {
  data$: any;
}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature', 'ɵɵpipe'],
  },

  // ==========================================================================
  // Real-World Use Cases
  // ==========================================================================

  {
    type: 'full-transform',
    name: 'host-directives-ui-component',
    category: 'host-directives',
    description: 'UI component with common directives (tooltip, accessibility)',
    className: 'UIButtonComponent',
    sourceCode: `
import { Component, Directive, Input } from '@angular/core';

@Directive({ selector: '[tooltip]', standalone: true })
export class TooltipDirective {
  @Input() tooltipText: string = '';
}

@Directive({ selector: '[ripple]', standalone: true })
export class RippleDirective {}

@Directive({ selector: '[aria]', standalone: true })
export class AriaDirective {
  @Input() label: string = '';
  @Input() describedBy: string = '';
}

@Component({
  selector: 'app-ui-button',
  standalone: true,
  hostDirectives: [
    { directive: TooltipDirective, inputs: ['tooltipText: tooltip'] },
    RippleDirective,
    { directive: AriaDirective, inputs: ['label: ariaLabel', 'describedBy: ariaDescribedBy'] }
  ],
  template: \`
    <button [disabled]="disabled">
      <ng-content></ng-content>
    </button>
  \`
})
export class UIButtonComponent {
  disabled = false;
}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature', 'ɵɵprojection'],
  },

  {
    type: 'full-transform',
    name: 'host-directives-form-field',
    category: 'host-directives',
    description: 'Form field with validation and accessibility directives',
    className: 'FormFieldComponent',
    sourceCode: `
import { Component, Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[validation]', standalone: true })
export class ValidationDirective {
  @Input() isRequired: boolean = false;
  @Input() minLen: number = 0;
  @Input() maxLen: number = 100;
  @Output() onError = new EventEmitter<string>();
}

@Directive({ selector: '[formFieldAccessibility]', standalone: true })
export class FormFieldAccessibilityDirective {
  @Input() id: string = '';
}

@Component({
  selector: 'app-form-field',
  standalone: true,
  hostDirectives: [
    {
      directive: ValidationDirective,
      inputs: ['isRequired: required', 'minLen: minLength', 'maxLen: maxLength'],
      outputs: ['onError: validationError']
    },
    { directive: FormFieldAccessibilityDirective, inputs: ['id: fieldId'] }
  ],
  template: \`
    <label>{{ label }}</label>
    <input [value]="value" (input)="onInput($event)" />
    @if (error) {
      <span class="error">{{ error }}</span>
    }
  \`
})
export class FormFieldComponent {
  label = '';
  value = '';
  error = '';
  onInput(event: Event) {}
}
`.trim(),
    expectedFeatures: [
      'ɵɵdefineComponent',
      'ɵɵHostDirectivesFeature',
      'ɵɵlistener',
      'ɵɵconditional',
    ],
  },

  {
    type: 'full-transform',
    name: 'host-directives-interactive-card',
    category: 'host-directives',
    description: 'Interactive card with multiple behavioral directives',
    className: 'InteractiveCardComponent',
    sourceCode: `
import { Component, Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[hoverEffect]', standalone: true })
export class HoverEffectDirective {
  @Input() scale: number = 1.05;
}

@Directive({ selector: '[clickable]', standalone: true })
export class ClickableDirective {
  @Output() onClick = new EventEmitter<void>();
}

@Directive({ selector: '[keyboardNavigable]', standalone: true })
export class KeyboardNavigableDirective {
  @Input() keyboardTabIndex: number = 0;
  @Output() onEnter = new EventEmitter<void>();
  @Output() onEscape = new EventEmitter<void>();
}

@Component({
  selector: 'app-interactive-card',
  standalone: true,
  hostDirectives: [
    { directive: HoverEffectDirective, inputs: ['scale: hoverScale'] },
    { directive: ClickableDirective, outputs: ['onClick: cardClick'] },
    {
      directive: KeyboardNavigableDirective,
      inputs: ['keyboardTabIndex: tabIndex'],
      outputs: ['onEnter: enterPressed', 'onEscape: escapePressed']
    }
  ],
  template: \`
    <div class="card">
      <header>{{ title }}</header>
      <main><ng-content></ng-content></main>
      <footer>
        @if (showActions) {
          <ng-content select="[card-actions]"></ng-content>
        }
      </footer>
    </div>
  \`
})
export class InteractiveCardComponent {
  title = '';
  showActions = true;
}
`.trim(),
    expectedFeatures: [
      'ɵɵdefineComponent',
      'ɵɵHostDirectivesFeature',
      'ɵɵprojectionDef',
      'ɵɵprojection',
      'ɵɵconditional',
    ],
  },

  {
    type: 'full-transform',
    name: 'host-directives-data-grid',
    category: 'host-directives',
    description: 'Data grid component with sorting, selection, and virtualization directives',
    className: 'DataGridComponent',
    sourceCode: `
import { Component, Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[sortable]', standalone: true })
export class SortableDirective {
  @Input() currentSortColumn: string = '';
  @Input() currentSortDirection: 'asc' | 'desc' = 'asc';
  @Output() onSortChange = new EventEmitter<{column: string, direction: string}>();
}

@Directive({ selector: '[selectable]', standalone: true })
export class SelectableDirective {
  @Input() mode: 'single' | 'multiple' = 'single';
  @Input() selected: any[] = [];
  @Output() onSelectionChange = new EventEmitter<any[]>();
}

@Directive({ selector: '[virtualScroll]', standalone: true })
export class VirtualScrollDirective {
  @Input() rowHeight: number = 40;
  @Input() scrollBuffer: number = 5;
}

@Component({
  selector: 'app-data-grid',
  standalone: true,
  hostDirectives: [
    {
      directive: SortableDirective,
      inputs: ['currentSortColumn: sortColumn', 'currentSortDirection: sortDirection'],
      outputs: ['onSortChange: sortChange']
    },
    {
      directive: SelectableDirective,
      inputs: ['mode: selectionMode', 'selected: selectedItems'],
      outputs: ['onSelectionChange: selectionChange']
    },
    {
      directive: VirtualScrollDirective,
      inputs: ['rowHeight: itemHeight', 'scrollBuffer: bufferSize']
    }
  ],
  template: \`
    <table>
      <thead>
        <tr>
          @for (col of columns; track col.key) {
            <th>{{ col.label }}</th>
          }
        </tr>
      </thead>
      <tbody>
        @for (row of visibleRows; track row.id) {
          <tr [class.selected]="isSelected(row)">
            @for (col of columns; track col.key) {
              <td>{{ row[col.key] }}</td>
            }
          </tr>
        }
      </tbody>
    </table>
  \`
})
export class DataGridComponent {
  columns: {key: string, label: string}[] = [];
  visibleRows: any[] = [];
  isSelected(row: any) { return false; }
}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature', 'ɵɵrepeaterCreate'],
  },

  // ==========================================================================
  // Edge Cases
  // ==========================================================================

  {
    type: 'full-transform',
    name: 'host-directives-empty-mappings',
    category: 'host-directives',
    description: 'Host directive with empty input/output arrays',
    className: 'EmptyMappingsComponent',
    sourceCode: `
import { Component, Directive } from '@angular/core';

@Directive({ selector: '[basic]', standalone: true })
export class BasicDirective {}

@Component({
  selector: 'app-empty-mappings',
  standalone: true,
  hostDirectives: [{ directive: BasicDirective, inputs: [], outputs: [] }],
  template: \`<div>Empty mappings</div>\`
})
export class EmptyMappingsComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  {
    type: 'full-transform',
    name: 'host-directives-forwarded-all',
    category: 'host-directives',
    description: 'Host directive forwarding all inputs and outputs',
    className: 'ForwardAllComponent',
    sourceCode: `
import { Component, Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[fullFeatured]', standalone: true })
export class FullFeaturedDirective {
  @Input() input1: string = '';
  @Input() input2: number = 0;
  @Input() input3: boolean = false;
  @Output() output1 = new EventEmitter<string>();
  @Output() output2 = new EventEmitter<number>();
}

@Component({
  selector: 'app-forward-all',
  standalone: true,
  hostDirectives: [{
    directive: FullFeaturedDirective,
    inputs: ['input1', 'input2', 'input3'],
    outputs: ['output1', 'output2']
  }],
  template: \`<div>All forwarded</div>\`
})
export class ForwardAllComponent {}
`.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵɵHostDirectivesFeature'],
  },

  // ==========================================================================
  // Host Directives on Directives (not just Components)
  // ==========================================================================

  {
    type: 'full-transform',
    name: 'directive-host-directive-input-mapping',
    category: 'host-directives',
    description: 'Directive with host directive input mappings (issue #67)',
    className: 'UnityTooltipTrigger',
    sourceCode: `
import { Directive, Input } from '@angular/core';

@Directive({ selector: '[brnTooltipTrigger]', standalone: true })
export class BrnTooltipTrigger {
  @Input() brnTooltipTrigger: string = '';
}

@Directive({
  selector: '[uTooltip]',
  standalone: true,
  hostDirectives: [{
    directive: BrnTooltipTrigger,
    inputs: ['brnTooltipTrigger: uTooltip']
  }]
})
export class UnityTooltipTrigger {}
`.trim(),
    expectedFeatures: ['ɵɵdefineDirective', 'ɵɵHostDirectivesFeature'],
  },

  {
    type: 'full-transform',
    name: 'directive-host-directive-output-mapping',
    category: 'host-directives',
    description: 'Directive with host directive output mappings',
    className: 'MyWrapperDirective',
    sourceCode: `
import { Directive, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[trackable]', standalone: true })
export class TrackableDirective {
  @Output() trackClick = new EventEmitter<void>();
}

@Directive({
  selector: '[myWrapper]',
  standalone: true,
  hostDirectives: [{
    directive: TrackableDirective,
    outputs: ['trackClick: clicked']
  }]
})
export class MyWrapperDirective {}
`.trim(),
    expectedFeatures: ['ɵɵdefineDirective', 'ɵɵHostDirectivesFeature'],
  },

  {
    type: 'full-transform',
    name: 'directive-host-directive-input-output-mapping',
    category: 'host-directives',
    description: 'Directive with both input and output host directive mappings',
    className: 'EnhancedDirective',
    sourceCode: `
import { Directive, Input, Output, EventEmitter } from '@angular/core';

@Directive({ selector: '[base]', standalone: true })
export class BaseDirective {
  @Input() baseValue: string = '';
  @Output() baseChange = new EventEmitter<string>();
}

@Directive({
  selector: '[enhanced]',
  standalone: true,
  hostDirectives: [{
    directive: BaseDirective,
    inputs: ['baseValue: value'],
    outputs: ['baseChange: valueChange']
  }]
})
export class EnhancedDirective {}
`.trim(),
    expectedFeatures: ['ɵɵdefineDirective', 'ɵɵHostDirectivesFeature'],
  },
]
