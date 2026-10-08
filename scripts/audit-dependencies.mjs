import { spawnSync } from 'node:child_process'
import { patches, verifyPatches } from './patch-dependencies.mjs'

verifyPatches()
const result = spawnSync('npm', ['audit', '--json'], { encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 })
if (result.error) throw result.error
const report = JSON.parse(result.stdout)
if (report.error || !report.vulnerabilities) throw new Error('npm advisory check was unavailable; refusing to pass')
const reviewed = new Set(patches.map(patch => patch.advisory))
const remaining = []
for (const [name, vulnerability] of Object.entries(report.vulnerabilities)) {
  for (const via of vulnerability.via) {
    if (typeof via !== 'object') continue
    const id = via.url?.split('/').pop()
    const patch = patches.find(item => item.advisory === id && item.package === name)
    const covered = patch && vulnerability.nodes.length === 1 && vulnerability.nodes[0] === `node_modules/${name}`
    if (!covered) remaining.push({ package: name, advisory: id, severity: via.severity })
  }
}
if (remaining.length) {
  console.error(JSON.stringify({ unpatched_advisories: remaining }, null, 2))
  process.exitCode = 1
} else console.log(JSON.stringify({ ok: true, locally_patched_advisories: [...reviewed] }))
