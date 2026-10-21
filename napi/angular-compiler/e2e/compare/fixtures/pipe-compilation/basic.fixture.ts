/**
 * Pipe compilation fixtures.
 *
 * Tests @Pipe decorator compilation via full file transformation.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    type: 'full-transform',
    name: 'pure-pipe',
    category: 'pipe-compilation',
    description: 'Basic pure pipe compilation',
    className: 'MyPurePipe',
    sourceCode: `
import { Pipe, PipeTransform } from '@angular/core';

@Pipe({
  name: 'myPure',
  pure: true,
  standalone: true,
})
export class MyPurePipe implements PipeTransform {
  transform(value: string): string {
    return value.toUpperCase();
  }
}
    `.trim(),
    expectedFeatures: ['definePipe'],
  },
  {
    type: 'full-transform',
    name: 'impure-pipe',
    category: 'pipe-compilation',
    description: 'Impure pipe compilation',
    className: 'MyImpurePipe',
    sourceCode: `
import { Pipe, PipeTransform } from '@angular/core';

@Pipe({
  name: 'myImpure',
  pure: false,
  standalone: true,
})
export class MyImpurePipe implements PipeTransform {
  transform(value: any[]): any[] {
    return [...value].reverse();
  }
}
    `.trim(),
    expectedFeatures: ['definePipe'],
  },
  {
    type: 'full-transform',
    name: 'non-standalone-pipe',
    category: 'pipe-compilation',
    description: 'Non-standalone pipe compilation (Angular v18 style)',
    className: 'LegacyPipe',
    sourceCode: `
import { Pipe, PipeTransform } from '@angular/core';

@Pipe({
  name: 'legacy',
  standalone: false,
})
export class LegacyPipe implements PipeTransform {
  transform(value: string): string {
    return value;
  }
}
    `.trim(),
    expectedFeatures: ['definePipe'],
  },
  {
    type: 'full-transform',
    name: 'minimal-pipe',
    category: 'pipe-compilation',
    description: 'Pipe with minimal configuration',
    className: 'MinimalPipe',
    sourceCode: `
import { Pipe, PipeTransform } from '@angular/core';

@Pipe({ name: 'minimal' })
export class MinimalPipe implements PipeTransform {
  transform(value: any): any {
    return value;
  }
}
    `.trim(),
    expectedFeatures: ['definePipe'],
  },
  {
    type: 'full-transform',
    name: 'pipe-extends-base-without-constructor',
    category: 'pipe-compilation',
    description: 'Pipe that extends a base class without a constructor inherits its factory',
    className: 'DerivedPipe',
    sourceCode: `
import { Pipe, PipeTransform } from '@angular/core';

class BasePipe {}

@Pipe({ name: 'derived' })
export class DerivedPipe extends BasePipe implements PipeTransform {
  transform(value: any): any {
    return value;
  }
}
    `.trim(),
    expectedFeatures: ['definePipe', 'ɵɵgetInheritedFactory'],
  },
]
