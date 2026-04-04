/**
 * Regression fixture: Non-exported class field declarations
 *
 * This tests the difference in how oxc and ng handle non-exported classes
 * that appear alongside Angular components. OXC strips uninitialized fields
 * (matching useDefineForClassFields:false, Angular's default), but the
 * comparison test's TS compiler uses useDefineForClassFields:true (ESNext default)
 * which preserves them as bare declarations. In real Angular projects both agree.
 *
 * Found in bitwarden-clients project in files like:
 * - import-chrome.component.ts (ChromeLogin class)
 * - send-add-edit.component.ts (QueryParams class)
 * - members.component.ts (MembersTableDataSource class)
 */
import type { Fixture } from '../types.js'

const sourceCode = `
import { Component } from '@angular/core';

@Component({
  selector: 'app-test',
  standalone: true,
  template: '<div>{{ helper.value }}</div>',
})
export class TestComponent {
  helper = new HelperClass('test');
}

// Non-exported helper class - this is what differs between compilers
class HelperClass {
  name: string;
  url: string;
  value: number;

  constructor(input: string) {
    this.name = input;
    this.url = 'https://example.com/' + input;
    this.value = input.length;
  }
}
`.trim()

const fixture: Fixture = {
  type: 'full-transform',
  name: 'bitwarden-nonexported-class',
  category: 'regressions',
  description:
    'Non-exported helper class field declarations differ between oxc and ng (cosmetic difference)',
  className: 'TestComponent',
  sourceCode,
  expectedFeatures: ['ɵɵdefineComponent', 'ɵɵtext'],
  // OXC strips uninitialized fields (useDefineForClassFields:false behavior) but the
  // comparison test's TS compiler uses useDefineForClassFields:true (ESNext default),
  // which preserves them as `name;` declarations. In real Angular projects both compilers
  // agree because tsconfig sets useDefineForClassFields:false.
  skip: true,
  skipReason:
    'Known cosmetic difference: comparison test uses useDefineForClassFields:true (ESNext default) while OXC strips uninitialized fields (useDefineForClassFields:false, Angular default). Both match in real Angular projects.',
}

export const fixtures = [fixture]
