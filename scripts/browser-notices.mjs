// Include the actual licenses of libraries that Rollup places in browser
// chunks. Source repository dependencies retain their upstream license files.
import { readFileSync, readdirSync, existsSync } from 'node:fs'
import { join } from 'node:path'

const roots = new Set()
export function browserNotices(collectOnly = false) {
  return {
    name: 'interdeck-browser-notices',
    buildStart() { if (!collectOnly) roots.clear() },
    generateBundle(_options, bundle) {
      for (const output of Object.values(bundle)) {
        if (output.type !== 'chunk') continue
        for (const id of Object.keys(output.modules)) {
          const matches = [...id.matchAll(/\/node_modules\/(?:@[^/]+\/)?[^/]+/g)]
          const last = matches.at(-1)
          if (last) roots.add(id.slice(0, last.index + last[0].length).replace(/^\0/, ''))
        }
      }
      if (collectOnly) return
      const notices = []
      for (const root of [...roots].sort()) {
        if (!existsSync(join(root, 'package.json'))) continue
        const metadata = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
        const files = readdirSync(root).filter(name => /^(license|licence|copying|copyright|notice)([._-]|$)/i.test(name))
        if (!files.length) throw new Error(`Missing bundled license notice: ${metadata.name}@${metadata.version}`)
        notices.push(`${metadata.name}@${metadata.version}\nLicense: ${metadata.license || 'see upstream notice'}\n${files.sort().map(name => `${name}\n${readFileSync(join(root, name), 'utf8')}`).join('\n\n')}`)
      }
      this.emitFile({ type: 'asset', fileName: 'BROWSER_THIRD_PARTY_NOTICES.txt', source: notices.join('\n\n----------------------------------------\n\n') })
    },
  }
}
