export function Log(message: string) {
  return function <T extends new (...args: any[]) => any>(target: T): T {
    console.log(message)
    return target
  }
}
