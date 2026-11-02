// Checks gc-lane.js: required args.session and args.paths, no literal paths in prompts.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const src = readFileSync(join(here, '..', 'gc-lane.js'), 'utf8').replace('export const meta', 'const meta')
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor
const fn = new AsyncFunction('args', 'agent', 'parallel', 'log', src)
async function run(args, agent) {
  return fn(args, agent ?? (async () => null), async (fs) => Promise.all(fs.map((f) => f())), () => {})
}

const P = { repo: '/r', tools: '/t', night: '/n', wt: '/w' }
const S = 'https://claude.ai/code/session_TESTabcd123'
let n = 0

await assert.rejects(run({ lane: 'x', prds: [], paths: P }), /args\.session is required/); n++
await assert.rejects(run({ lane: 'x', prds: [], session: S }), /args\.paths is required/); n++
assert.deepEqual(await run({ lane: 'x', prds: [], session: S, paths: P }), []); n++

const seen = []
await run({ lane: 'x', session: S, paths: P, prds: [{ id: 'PRD-99', slug: 'prd-99-x', file: 'PRD-99.md', core: false }] },
  async (p) => { seen.push(p); return { done: true, notes: '', tasks: [] } })
assert.match(seen[0], /Claude-Session: https:\/\/claude\.ai\/code\/session_TESTabcd123/)
assert.match(seen[0], /node \/t\/box\.mjs run/)
assert.match(seen[0], /\/n\/lanes\/x\.log/)
assert.doesNotMatch(seen.join('\n'), /\/root\/|\/home\/user|session_01V2/)
n++

console.log(`test_gc_lane: ${n} passed`)
