const headmatterPattern = /^---[\t ]*\r?\n([\s\S]*?)\r?\n---(?:[\t ]*\r?\n|$)/

export function readConfiguredTheme(source) {
  const headmatter = source.match(headmatterPattern)
  if (!headmatter) return 'default'
  const theme = headmatter[1].match(/^theme\s*:\s*['"]?([^'"\s#]+)['"]?\s*(?:#.*)?$/m)
  return theme?.[1] || 'default'
}

export function repairUnsafeHeadmatterTitle(source) {
  const headmatter = String(source).match(headmatterPattern)
  if (!headmatter) return source
  const repairedBody = headmatter[1].replace(
    /^title:([^\r\n]*)$/m,
    (_line, rawValue) => {
      const commentAt = rawValue.indexOf(' #')
      const raw = (commentAt >= 0 ? rawValue.slice(0, commentAt) : rawValue).trim()
      const comment = commentAt >= 0 ? rawValue.slice(commentAt) : ''
      let value = raw
      try {
        const decoded = JSON.parse(raw)
        if (typeof decoded === 'string') value = decoded
      }
      catch {
        // The repair is specifically for a malformed generated scalar.
      }
      value = value.replaceAll('\\"', '"').replaceAll('"', '').trim()
      return `title: ${JSON.stringify(value)}${comment}`
    },
  )
  if (repairedBody === headmatter[1]) return source
  return `${source.slice(0, headmatter.index)}${headmatter[0].replace(headmatter[1], repairedBody)}${source.slice((headmatter.index || 0) + headmatter[0].length)}`
}

export function setConfiguredTheme(source, theme) {
  const headmatter = source.match(headmatterPattern)
  if (!headmatter) return `---\ntheme: ${theme}\n---\n\n${source}`
  const body = /^theme\s*:[^\r\n]*$/m.test(headmatter[1])
    ? headmatter[1].replace(/^theme\s*:[^\r\n]*$/m, `theme: ${theme}`)
    : `theme: ${theme}\n${headmatter[1]}`
  const remainder = source.slice(headmatter[0].length).replace(/^\r?\n*/, '')
  return `---\n${body.trimEnd()}\n---\n\n${remainder}`
}

export function buildSlidevExportUrl(accessUrl, baseUrl) {
  const url = new URL(accessUrl, baseUrl)
  url.pathname = `${url.pathname.replace(/\/$/, '')}/export`
  return url.toString()
}
