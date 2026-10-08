import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import dotenv from 'dotenv'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
dotenv.config({ path: resolve(root, '.env') })

const databaseUrl = process.env.DATABASE_URL || 'postgresql://postgres:postgres@127.0.0.1:55432/interdeck'
const hostname = new URL(databaseUrl).hostname
if (!['127.0.0.1', 'localhost', '::1'].includes(hostname) && process.env.ALLOW_REMOTE_LOAD_SEED !== '1') {
  console.error(`Refusing to replace the load fixture on remote database host ${hostname}. Set ALLOW_REMOTE_LOAD_SEED=1 only for an isolated load-test database.`)
  process.exit(2)
}

const result = spawnSync('psql', [
  databaseUrl,
  '-v', 'ON_ERROR_STOP=1',
  '-f', resolve(root, 'scripts/load-fixture.sql'),
], { stdio: 'inherit' })

if (result.error) throw result.error
process.exit(result.status ?? 1)
