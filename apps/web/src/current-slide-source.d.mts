export interface CurrentSlideContent {
  slideNumber: number
  slideKey: string
  start: number
  end: number
  value: string
}

export function findCurrentSlideContent(source: string, slideNumber: number): CurrentSlideContent | null
export function findSlideNumberByKey(source: string, slideKey?: string): number | null
export function findProposalSlide(source: string, proposedSource: string, preferredKey: string | undefined, currentSlideNumber: number): { slideKey: string; slideNumber: number } | null
export function replaceCurrentSlideContent(source: string, slideNumber: number, value: string): string
