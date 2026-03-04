export function Log(message: string) {
  console.log(`[Log Decorator] Applying decorator: ${message}`)

  return function <T extends new (...args: any[]) => any>(target: T) {
    console.log(`[Log Decorator] Decorating class: ${target.name}`)

    return class extends target {
      constructor(...args: any[]) {
        super(...args)
        console.log(`[Log Decorator] Instance created: ${target.name} — ${message}`)
      }
    }
  }
}
