import { spawnSync } from 'node:child_process'
const manifest = 'services/app/Cargo.toml'
const tree = spawnSync('cargo', ['tree', '--manifest-path', manifest, '--edges', 'normal,build', '--prefix', 'none'], { encoding: 'utf8' })
if (tree.status !== 0) throw new Error(tree.stderr || 'Cargo dependency graph was unavailable')
if (/^rsa v/m.test(tree.stdout)) throw new Error('RUSTSEC-2023-0071 became an active dependency; its audit exception is no longer valid')
// sqlx records its optional MySQL driver in Cargo.lock. The PostgreSQL-only
// application does not build rsa. Never ignore this advisory if that changes.
const audit = spawnSync('cargo', ['audit', '--file', 'services/app/Cargo.lock', '--ignore', 'RUSTSEC-2023-0071'], { stdio: 'inherit' })
if (audit.error) throw audit.error
process.exitCode = audit.status ?? 1
