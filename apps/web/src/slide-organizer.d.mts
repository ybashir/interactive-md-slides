import type { ParsedSlide } from './types'

export interface OrganizedSlide {
  key: string
  number: number
  title: string
  blockStart: number
  blockEnd: number
  interactionCount: number
  fixed: boolean
}

export function listOrganizedSlides(source: string, parsedSlides?: ParsedSlide[]): OrganizedSlide[]
export function reorderOrganizedSlides(source: string, orderedKeys: string[]): string
export function deleteOrganizedSlide(source: string, slideKey: string): string
export function duplicateOrganizedSlide(source: string, slideKey: string): { source: string; slideKey: string | null }
export function insertOrganizedSlide(source: string, afterSlideKey: string): { source: string; slideKey: string | null }
