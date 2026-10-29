import { describe, expect, test } from 'vitest'

import type { FactoryCompileInput, DependencyMetadata } from '../index.js'
import { compileFactory } from '../index.js'

describe('compileFactory', () => {
  test('compiles simple factory with no dependencies', async () => {
    const input: FactoryCompileInput = {
      name: 'MyService',
      target: 'Injectable',
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('MyService_Factory')
    expect(result.code).toContain('__ngFactoryType__')
    expect(result.code).toContain('new')
    expect(result.code).toContain('MyService')
  })

  test('compiles factory with dependencies', async () => {
    const deps: DependencyMetadata[] = [
      { token: 'HttpClient' },
      { token: 'Logger', optional: true },
    ]

    const input: FactoryCompileInput = {
      name: 'DataService',
      target: 'Injectable',
      deps,
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('DataService_Factory')
    expect(result.code).toContain('inject')
    expect(result.code).toContain('HttpClient')
    expect(result.code).toContain('Logger')
  })

  test('compiles factory for component', async () => {
    const deps: DependencyMetadata[] = [{ token: 'ElementRef' }]

    const input: FactoryCompileInput = {
      name: 'MyComponent',
      target: 'Component',
      deps,
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('MyComponent_Factory')
    // Components use directiveInject instead of inject
    expect(result.code).toContain('directiveInject')
  })

  test('compiles factory for directive', async () => {
    const input: FactoryCompileInput = {
      name: 'MyDirective',
      target: 'Directive',
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('MyDirective_Factory')
  })

  test('compiles factory for pipe', async () => {
    const input: FactoryCompileInput = {
      name: 'MyPipe',
      target: 'Pipe',
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('MyPipe_Factory')
  })

  test('compiles factory for NgModule', async () => {
    const input: FactoryCompileInput = {
      name: 'AppModule',
      target: 'NgModule',
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('AppModule_Factory')
  })

  test('compiles inherited factory (deps_kind: None)', async () => {
    const input: FactoryCompileInput = {
      name: 'ChildClass',
      target: 'Component',
      depsKind: 'None',
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('ChildClass_Factory')
    // Inherited factories use getInheritedFactory
    expect(result.code).toContain('getInheritedFactory')
    expect(result.code).toContain('_BaseFactory')
  })

  test('compiles invalid factory (deps_kind: Invalid)', async () => {
    const input: FactoryCompileInput = {
      name: 'BrokenClass',
      target: 'Injectable',
      depsKind: 'Invalid',
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('BrokenClass_Factory')
    expect(result.code).toContain('invalidFactory')
  })

  test('compiles factory with @Self dependency', async () => {
    const deps: DependencyMetadata[] = [{ token: 'LocalService', self: true }]

    const input: FactoryCompileInput = {
      name: 'MyService',
      target: 'Injectable',
      deps,
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('MyService_Factory')
    // @Self sets a flag bit (2)
    expect(result.code).toMatch(/inject.*LocalService.*2/)
  })

  test('compiles factory with @SkipSelf dependency', async () => {
    const deps: DependencyMetadata[] = [{ token: 'ParentService', skipSelf: true }]

    const input: FactoryCompileInput = {
      name: 'MyService',
      target: 'Injectable',
      deps,
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('MyService_Factory')
    // @SkipSelf sets a flag bit
    expect(result.code).toMatch(/inject.*ParentService.*4/)
  })

  test('compiles factory with @Host dependency', async () => {
    const deps: DependencyMetadata[] = [{ token: 'HostService', host: true }]

    const input: FactoryCompileInput = {
      name: 'MyService',
      target: 'Injectable',
      deps,
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('MyService_Factory')
    // @Host sets a flag bit
    expect(result.code).toMatch(/inject.*HostService.*1/)
  })

  test('compiles factory with @Optional dependency', async () => {
    const deps: DependencyMetadata[] = [{ token: 'OptionalService', optional: true }]

    const input: FactoryCompileInput = {
      name: 'MyService',
      target: 'Injectable',
      deps,
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('MyService_Factory')
    // @Optional sets a flag bit (8)
    expect(result.code).toMatch(/inject.*OptionalService.*8/)
  })

  test('compiles factory with @Attribute dependency', async () => {
    const deps: DependencyMetadata[] = [{ token: 'color', attributeNameType: 'color' }]

    const input: FactoryCompileInput = {
      name: 'MyDirective',
      target: 'Directive',
      deps,
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('MyDirective_Factory')
    // @Attribute uses injectAttribute
    expect(result.code).toContain('injectAttribute')
  })

  test('compiles factory with invalid dependency (null token)', async () => {
    const deps: DependencyMetadata[] = [{ token: undefined }]

    const input: FactoryCompileInput = {
      name: 'MyService',
      target: 'Injectable',
      deps,
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('MyService_Factory')
    // Like ngtsc, an invalid dependency makes the whole factory invalid
    expect(result.code).toContain('ɵɵinvalidFactory()')
    expect(result.code).not.toContain('invalidFactoryDep')
  })

  test('returns error for invalid target', async () => {
    const input: FactoryCompileInput = {
      name: 'MyClass',
      target: 'InvalidTarget' as any,
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(1)
    expect(result.errors[0].message).toContain('Invalid factory target')
    expect(result.code).toBe('')
  })

  test('returns error for invalid deps_kind', async () => {
    const input: FactoryCompileInput = {
      name: 'MyClass',
      depsKind: 'InvalidKind' as any,
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(1)
    expect(result.errors[0].message).toContain('Invalid deps_kind')
    expect(result.code).toBe('')
  })

  test('defaults to Injectable target when not specified', async () => {
    const input: FactoryCompileInput = {
      name: 'MyService',
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('MyService_Factory')
    // Injectable uses inject (not directiveInject)
    expect(result.code).not.toContain('directiveInject')
  })

  test('pipe factory uses directiveInject with FOR_PIPE flag', async () => {
    const deps: DependencyMetadata[] = [{ token: 'SomeService' }]

    const input: FactoryCompileInput = {
      name: 'MyPipe',
      target: 'Pipe',
      deps,
    }

    const result = await compileFactory(input)
    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain('directiveInject')
    // Pipes use FOR_PIPE flag (16)
    expect(result.code).toMatch(/directiveInject.*SomeService.*16/)
  })
})
