export function audienceCanSeeResults(interaction, hasResponded) {
  if (interaction.kind === 'ranked-list') return interaction.phase === 'ranked'
  const policy = interaction.config.results
  if (policy === 'always' || policy === 'live') return true
  if (policy === 'hidden' || policy === 'presenter') return false
  if (policy === 'manual' || policy === 'on-close') return interaction.results_revealed
  return hasResponded
}

export function mergeAudienceSnapshot(previous, shared, sequence, ownResponses) {
  const sameEpoch = previous?.result_epoch === shared.result_epoch
  if (!sameEpoch) ownResponses.clear()
  const priorById = new Map(sameEpoch ? previous.interactions.map(item => [item.id, item]) : [])
  const interactions = shared.interactions.map(item => {
    const prior = priorById.get(item.id)
    const response = ownResponses.get(item.id)
    const result = audienceCanSeeResults(item, Boolean(response))
      ? {
          ...(item.result?.hidden ? prior?.result || { hidden: true } : item.result || {}),
          ...(prior?.result?.correct_option_id ? { correct_option_id: prior.result.correct_option_id } : {}),
        }
      : { hidden: true }
    return { ...item, response, result }
  })
  const publicIds = new Set(shared.questions.map(item => item.id))
  const ownQuestions = sameEpoch ? previous.questions.filter(item => item.mine && !publicIds.has(item.id)) : []
  return {
    ...shared,
    sequence,
    audience_session_active: previous?.audience_session_active || false,
    interactions,
    questions: [...shared.questions, ...ownQuestions],
  }
}
