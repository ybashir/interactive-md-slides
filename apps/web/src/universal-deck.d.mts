export interface UniversalSegment {
  kind: 'html' | 'interaction' | 'chart' | 'qr' | 'audience-count' | 'live-qa' | 'live-summary'
  html?: string
  definition?: Record<string, any>
  live?: Record<string, any>
  display?: string
  reveal?: string
  clickStart?: number
  bodyHtml?: string
  spec?: import('./chart-spec.mjs').ChartSpec
  size?: number
  label?: string
  max?: number
  show?: string
}

export interface UniversalSlide {
  index: number
  title: string
  frontmatter: Record<string, any>
  layout: string
  classes: string
  background: string
  transition: string
  slots: Record<string, string>
}

export function parseUniversalDeck(source: string): {
  theme: string
  title: string
  styles: string
  scopedStyles: Array<{ slideIndex: number; css: string }>
  slides: UniversalSlide[]
}

export function renderUniversalSlide(
  slide: UniversalSlide,
  definitions: Array<Record<string, any>>,
  liveInteractions: Array<Record<string, any>>,
  clickStep: number,
): { slots: Record<string, UniversalSegment[]>; clicksTotal: number; visibleElementInteractionIds: string[] }

export function applyClickGroups(
  html: string,
  clickStep: number,
  state: { next: number },
): string

export function markClickSyntax(source: string): string
