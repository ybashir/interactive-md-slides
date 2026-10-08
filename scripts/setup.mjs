import { randomBytes } from 'node:crypto'
import { readFile, writeFile } from 'node:fs/promises'

const template = await readFile(new URL('../.env.example', import.meta.url), 'utf8')
const contents = template
  .replace('INTERNAL_SERVICE_TOKEN=development-internal-token', `INTERNAL_SERVICE_TOKEN=${randomBytes(32).toString('hex')}`)
  .replace('SLIDEV_TOKEN_SECRET=development-slidev-token-secret', `SLIDEV_TOKEN_SECRET=${randomBytes(32).toString('hex')}`)
try {
  await writeFile(new URL('../.env', import.meta.url), contents, { flag: 'wx', mode: 0o600 })
  console.log('Created .env with independent service secrets. Set your Google OAuth credentials and creator access policy before starting.')
} catch (error) {
  if (error.code !== 'EEXIST') throw error
  console.log('.env already exists; preserved your configuration. See .env.example for new settings.')
}
