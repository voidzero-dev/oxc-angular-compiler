/**
 * The text to report for a compiler diagnostic. Rollup shows only this string, so the
 * help text is appended to it: for a template parse error that is where the error is
 * (`file:line:column`).
 */
export function describeDiagnostic(diagnostic: {
  message: string
  helpMessage?: string | null
}): string {
  return diagnostic.helpMessage
    ? `${diagnostic.message}\n${diagnostic.helpMessage}`
    : diagnostic.message
}
