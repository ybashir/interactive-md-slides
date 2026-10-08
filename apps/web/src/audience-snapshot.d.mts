import type { AudienceInteraction, LiveState } from './types'
export function audienceCanSeeResults(interaction: AudienceInteraction, hasResponded: boolean): boolean
export function mergeAudienceSnapshot(previous: LiveState | null, shared: LiveState, sequence: number, ownResponses: Map<string, Record<string, any>>): LiveState
