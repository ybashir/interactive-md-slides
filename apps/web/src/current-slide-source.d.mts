export interface CurrentSlideContent {
  slideNumber: number
  slideKey: string
  start: number
  end: number
  value: string
}

export function findCurrentSlideContent(source: string, slideNumber: number): CurrentSlideContent | null
export function findSlideNumberByKey(source: string, slideKey?: string): number | null
export function replaceCurrentSlideContent(source: string, slideNumber: number, value: string): string
