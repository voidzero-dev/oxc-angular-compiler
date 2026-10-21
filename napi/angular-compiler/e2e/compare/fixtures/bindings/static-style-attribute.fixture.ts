/**
 * Static `style` attributes, which the `parseExtractedStyles` phase splits into
 * style properties with Angular's `parse()` (style_parser), quirks included.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'style-attr-empty-value',
    category: 'bindings',
    description: 'Declarations with an empty value are kept',
    className: 'StyleAttrEmptyValueComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-style-attr-empty-value',
  standalone: true,
  template: \`
    <div style="color:;width: 10px">a</div>
    <span style="height:">b</span>
  \`,
})
export class StyleAttrEmptyValueComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵelementStart'],
  },
  {
    name: 'style-attr-stray-semicolons',
    category: 'bindings',
    description: 'A semicolon before a property name stays in the name',
    className: 'StyleAttrStraySemicolonsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-style-attr-stray-semicolons',
  standalone: true,
  template: \`
    <div style="color: red;;width: 10px">a</div>
    <span style="x;height: 5px">b</span>
  \`,
})
export class StyleAttrStraySemicolonsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵelementStart'],
  },
  {
    name: 'style-attr-quotes-and-parens',
    category: 'bindings',
    description: 'Quotes, escaped quotes and parentheses in style values',
    className: 'StyleAttrQuotesAndParensComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-style-attr-quotes-and-parens',
  standalone: true,
  template: \`
    <div style="background: url('a;b:c.png'); fontFamily: 'x;y'">a</div>
    <span style='content: "a\\\\"; color: red'>b</span>
  \`,
})
export class StyleAttrQuotesAndParensComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵelementStart'],
  },
]
