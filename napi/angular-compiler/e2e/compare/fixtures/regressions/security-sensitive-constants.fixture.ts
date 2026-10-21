/**
 * Security-sensitive static attributes.
 *
 * Constant values of security-sensitive attributes are wrapped in their trusted value
 * function (`ɵɵtrustConstantHtml` / `ɵɵtrustConstantResourceUrl`) in the consts array,
 * and static i18n attributes are wrapped in their sanitizer (`resolveI18nAttrSanitizers`).
 * Host attributes stay plain strings.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    // Port of r3_compiler_compliance/elements/security_sensitive_constant_attributes.
    name: 'security-sensitive-constant-attributes',
    category: 'regressions',
    description: 'Trusted value functions for security-sensitive constant attributes',
    className: 'SecuritySensitiveConstantAttributesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'my-component',
  standalone: true,
  template: \`
    <!-- A couple of security-sensitive attributes with constant values -->
    <embed src="https://angular.io/" />
    <iframe srcdoc="<h1>Angular</h1>"></iframe>
    <object data="https://angular.io/" codebase="/"></object>

    <!-- Repeated element to make sure attribute deduplication works properly -->
    <embed src="https://angular.io/" />

    <!-- Another element with a src attribute that is not security sensitive -->
    <img src="https://angular.io/" />
  \`,
})
export class SecuritySensitiveConstantAttributesComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵtrustConstantResourceUrl', 'ɵɵtrustConstantHtml'],
  },
  {
    name: 'security-sensitive-constant-templates',
    category: 'regressions',
    description:
      'Trusted constants on structural templates, ng-template, @if and SVG, with template-literal escapes',
    className: 'SecuritySensitiveConstantTemplatesComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { NgIf } from '@angular/common';

@Component({
  selector: 'app-security-sensitive-constant-templates',
  standalone: true,
  imports: [NgIf],
  template: \`
    <div *ngIf="show" innerHTML="<b>bold</b>"></div>
    <ng-template innerHTML="<i>italic</i>"></ng-template>
    @if (show) {
      <embed src="/a\\\${x}\\\`b" />
    }
    <svg><a href="/svg-link"></a></svg>
  \`,
})
export class SecuritySensitiveConstantTemplatesComponent {
  show = true;
}
    `.trim(),
    expectedFeatures: ['ɵɵtrustConstantResourceUrl', 'ɵɵtrustConstantHtml'],
  },
  {
    name: 'i18n-attribute-sanitizers',
    category: 'regressions',
    description: 'Static i18n attributes are wrapped in their sanitizer or ɵɵvalidateAttribute',
    className: 'I18nAttributeSanitizersComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';
import { NgIf } from '@angular/common';

@Component({
  selector: 'app-i18n-attribute-sanitizers',
  standalone: true,
  imports: [NgIf],
  template: \`
    <a href="https://x" i18n-href>x</a>
    <svg>
      <a href="https://y" i18n-href></a>
      <animate attributeName="x" i18n-attributeName></animate>
      <set to="x" i18n-to></set>
    </svg>
    <ng-template href="x" i18n-href></ng-template>
    <a *ngIf="show" href="https://z" i18n-href>z</a>
    <math><mi href="https://m" i18n-href></mi></math>
    <form action="/go" i18n-action></form>
    <area href="/a" i18n-href />
    <div title="plain" i18n-title></div>
  \`,
})
export class I18nAttributeSanitizersComponent {
  show = true;
}
    `.trim(),
    expectedFeatures: ['ɵɵsanitizeUrl', 'ɵɵvalidateAttribute'],
  },
  {
    // Port of r3_view_compiler_bindings/host_bindings/security_sensitive_constant_attributes.
    name: 'host-security-sensitive-constant-attributes',
    category: 'regressions',
    description: 'Host attributes get no trusted value functions',
    className: 'HostBindingDir',
    type: 'full-transform',
    sourceCode: `
import { Directive } from '@angular/core';

@Directive({
  selector: '[hostBindingDir]',
  host: { 'src': 'trusted', 'srcdoc': 'trusted' },
})
export class HostBindingDir {}

@Directive({
  selector: 'img',
  host: { 'src': 'trusted', 'srcdoc': 'trusted' },
})
export class HostBindingDir2 {}
    `.trim(),
    expectedFeatures: ['hostAttrs'],
  },
]
