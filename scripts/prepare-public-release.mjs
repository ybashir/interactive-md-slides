// Produce a new one-commit repository from reviewed, committed source. This
// never copies .git, ignored files, old refs, or historical blobs.
import { spawnSync } from 'node:child_process'
import { mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createHash } from 'node:crypto'

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, ...options })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(result.stderr || `${command} failed`)
  return result.stdout
}
if (run('git', ['status', '--porcelain']).trim()) throw new Error('Commit and review all source changes before preparing a public snapshot')
const sourceCommit = run('git', ['rev-parse', 'HEAD']).trim()
const destination = await mkdtemp(join(tmpdir(), 'interdeck-public-release-'))
const archive = run('git', ['archive', '--format=tar', 'HEAD'], { encoding: null })
run('tar', ['-xf', '-', '-C', destination], { input: archive, encoding: null })
const git = args => run('git', ['-c', 'core.hooksPath=/dev/null', ...args], { cwd: destination })
git(['init', '--initial-branch=main'])
git(['config', 'user.name', 'Yasser Bashir'])
git(['config', 'user.email', 'ybashir@users.noreply.github.com'])
git(['add', '--all'])
git(['commit', '-m', 'Initial open-source release'])
if (git(['rev-list', '--all', '--count']).trim() !== '1') throw new Error('Release history must contain exactly one commit')
run(process.env.GITLEAKS_BIN || 'gitleaks', ['git', '--redact=100', '--log-opts=--all'], { cwd: destination })
const publicCommit = git(['rev-parse', 'HEAD']).trim()
const tarball = `${destination}.tar.gz`
run('tar', ['-czf', tarball, '--exclude=.git', '-C', destination, '.'])
const sha256 = createHash('sha256').update(await readFile(tarball)).digest('hex')
await writeFile(`${tarball}.sha256`, `${sha256}  ${tarball}\n`)
console.log(JSON.stringify({ repository: destination, tarball, sha256, source_commit: sourceCommit, public_commit: publicCommit, history_commits: 1 }, null, 2))
