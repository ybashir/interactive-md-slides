import assert from 'node:assert/strict'
import test from 'node:test'
import { mergeAudienceSnapshot } from './audience-snapshot.mjs'

const interaction = (policy, result = { hidden: true }, revealed = false) => ({ id: policy, kind: 'poll', config: { results: policy }, results_revealed: revealed, result })
const state = (interactions, epoch = 1) => ({ interactions, result_epoch: epoch, audience_session_active: true, questions: [] })

test('shared state preserves only previously authorized after-vote results', () => {
  const previous = state([interaction('after-vote', { count: 2 })])
  const shared = state([interaction('after-vote')])
  const mine = new Map([['after-vote', { option_id: 'one' }]])
  assert.equal(mergeAudienceSnapshot(previous, shared, 4, mine).interactions[0].result.count, 2)
  assert.deepEqual(mergeAudienceSnapshot(previous, shared, 4, new Map()).interactions[0].result, { hidden: true })
})

test('a reset discards responses, protected tallies, answers, and pending questions from the previous epoch', () => {
  const previous = state([interaction('after-vote', { count: 8, correct_option_id: 'one' })])
  previous.questions.push({ id: 'private', mine: true })
  const responses = new Map([['after-vote', { option_id: 'one' }]])
  const merged = mergeAudienceSnapshot(previous, state([interaction('after-vote')], 2), 10, responses)
  assert.equal(responses.size, 0)
  assert.deepEqual(merged.interactions[0].result, { hidden: true })
  assert.deepEqual(merged.questions, [])
})

test('hidden, presenter-only, and unrevealed manual results remain hidden even after a response', () => {
  for (const policy of ['hidden', 'presenter', 'manual', 'on-close']) {
    const responses = new Map([[policy, { option_id: 'one' }]])
    const merged = mergeAudienceSnapshot(state([interaction(policy, { count: 5 })]), state([interaction(policy, { count: 9 })]), 2, responses)
    assert.deepEqual(merged.interactions[0].result, { hidden: true })
  }
})

test('public reveal updates tallies and deduplicates the participant’s published question', () => {
  const previous = state([interaction('manual')])
  previous.questions.push({ id: 'question', mine: true })
  const shared = state([interaction('manual', { count: 3 }, true)])
  shared.questions.push({ id: 'question', mine: false })
  const merged = mergeAudienceSnapshot(previous, shared, 3, new Map())
  assert.equal(merged.interactions[0].result.count, 3)
  assert.equal(merged.questions.length, 1)
})
