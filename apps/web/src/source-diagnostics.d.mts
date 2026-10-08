export interface SourceDiagnostic {
  message: string
  severity: 'error' | 'warning'
  startLine: number
  startColumn: number
  endLine: number
  endColumn: number
}

export function diagnoseSource(markdown: string): SourceDiagnostic[]
