import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
export const patches = JSON.parse(readFileSync(resolve(root, 'scripts/dependency-patches.json'), 'utf8'))
const hash = value => createHash('sha256').update(value).digest('hex')

export function verifyPatches(apply = false) {
  for (const patch of patches) {
    const folder = resolve(root, 'node_modules', patch.package)
    const installed = JSON.parse(readFileSync(resolve(folder, 'package.json'), 'utf8'))
    if (installed.version !== patch.version) throw new Error(`Review the ${patch.package} security patch for version ${installed.version}`)
    const path = resolve(folder, patch.path)
    const source = readFileSync(path, 'utf8')
    if (hash(source) === patch.patchedSha256) continue
    if (!apply || hash(source) !== patch.originalSha256) throw new Error(`Missing or unexpected ${patch.package} security patch`)
    const patched = source.replace(patch.before, patch.after)
    if (hash(patched) !== patch.patchedSha256) throw new Error(`Invalid ${patch.package} patch output`)
    writeFileSync(path, patched)
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  verifyPatches(true)
  console.log('Dependency security patches applied and verified')
}
