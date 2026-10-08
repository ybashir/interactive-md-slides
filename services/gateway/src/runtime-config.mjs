export function runtimeConfig(env) {
  const appEnv = env.APP_ENV || (env.NODE_ENV === 'production' ? 'production' : 'development')
  if (!['development', 'test', 'production'].includes(appEnv)) throw new Error('APP_ENV must be development, test, or production')
  const internalToken = env.INTERNAL_SERVICE_TOKEN || 'development-internal-token'
  const slidevSecret = env.SLIDEV_TOKEN_SECRET || 'development-slidev-token-secret'
  const trustProxy = Number(env.GATEWAY_TRUST_PROXY || '0')
  if (!Number.isInteger(trustProxy) || trustProxy < 0 || trustProxy > 5)
    throw new Error('GATEWAY_TRUST_PROXY must be a fixed number of trusted proxy hops between 0 and 5')
  const requestLimits = Object.fromEntries(Object.entries({ auth: 60, source: 60, export: 5000, static: 20000 }).map(([name, fallback]) => {
    const limit = Number(env[`GATEWAY_${name.toUpperCase()}_REQUESTS_PER_MINUTE`] || fallback)
    if (!Number.isSafeInteger(limit) || limit < 1) throw new Error(`Invalid gateway ${name} request limit`)
    return [name, limit]
  }))
  if (appEnv === 'production' && (!strongSecret(internalToken) || !strongSecret(slidevSecret) || internalToken === slidevSecret)) {
    throw new Error('Production requires distinct random INTERNAL_SERVICE_TOKEN and SLIDEV_TOKEN_SECRET values of at least 32 characters')
  }
  return { appEnv, internalToken, slidevSecret, trustProxy, requestLimits }
}

function strongSecret(value) {
  return value.length >= 32 && /^[\x21-\x7e]+$/.test(value)
    && !value.startsWith('development-') && !value.includes('replace-me') && new Set(value).size >= 8
}
