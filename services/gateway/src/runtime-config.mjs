export function runtimeConfig(env) {
  const appEnv = env.APP_ENV || (env.NODE_ENV === 'production' ? 'production' : 'development')
  if (!['development', 'test', 'production'].includes(appEnv)) throw new Error('APP_ENV must be development, test, or production')
  const internalToken = env.INTERNAL_SERVICE_TOKEN || 'development-internal-token'
  const slidevSecret = env.SLIDEV_TOKEN_SECRET || 'development-slidev-token-secret'
  if (appEnv === 'production' && (!strongSecret(internalToken) || !strongSecret(slidevSecret) || internalToken === slidevSecret)) {
    throw new Error('Production requires distinct random INTERNAL_SERVICE_TOKEN and SLIDEV_TOKEN_SECRET values of at least 32 characters')
  }
  return { appEnv, internalToken, slidevSecret }
}

function strongSecret(value) {
  return value.length >= 32 && /^[\x21-\x7e]+$/.test(value)
    && !value.startsWith('development-') && !value.includes('replace-me') && new Set(value).size >= 8
}
