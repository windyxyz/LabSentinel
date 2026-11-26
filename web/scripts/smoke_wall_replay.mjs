// Browser round trip for cursor replay against an isolated LabSentinel process.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { once } from 'node:events'
import { access, mkdir, open, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { join, resolve } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import { setTimeout as delay } from 'node:timers/promises'
import { chromium } from 'playwright'

const [stage, out, serverOverride, webOverride] = process.argv.slice(2)
assert(stage && out, 'Usage: node web/scripts/smoke_wall_replay.mjs <package> <new output directory> [server] [web dist]')
const pkg = resolve(stage)
const root = resolve(out)
const data = join(root, 'data')
const binary = resolve(serverOverride ?? join(pkg, 'labsentinel-server.exe'))
const web = resolve(webOverride ?? join(pkg, 'web'))
const dbPath = join(data, 'labsentinel.db')
const admin = randomBytes(32).toString('hex')
const portLease = createServer()
portLease.listen(0, '127.0.0.1')
await once(portLease, 'listening')
const { port } = portLease.address()
await new Promise(done => portLease.close(done))
const origin = 'http://127.0.0.1:' + port
const summary = { status: 'running' }
let server, exited, browser, db

async function request(path, method = 'GET', body) {
  const response = await fetch(origin + path, {
    method, signal: AbortSignal.timeout(5000),
    headers: { 'Content-Type': 'application/json', 'X-Admin-Token': admin },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  assert(response.ok, method + ' ' + path + ' returned ' + response.status)
  return response.json()
}

await Promise.all([binary, join(web, 'index.html')].map(file => access(file)))
await mkdir(root)
await mkdir(data)
try {
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(LS_|ORT_)/i.test(key)))
  Object.assign(env, { LS_HOST: '127.0.0.1', LS_PORT: String(port), LS_DATA_DIR: data,
    LS_DB_PATH: dbPath, LS_INSTALL_DIR: pkg, LS_WEB_DIST: web,
    LS_ADMIN_TOKEN: admin, LS_AUTH_READ_PROTECTED: '1', LS_GPU: '0', LS_LAUNCHER_NO_BROWSER: '1' })
  const log = await open(join(root, 'product.log'), 'w')
  try {
    server = spawn(binary, [], { cwd: root, env, windowsHide: true, stdio: ['ignore', log.fd, log.fd] })
    await once(server, 'spawn')
    exited = once(server, 'exit')
  } finally {
    await log.close()
  }
  const deadline = Date.now() + 20_000
  let ready = false
  while (Date.now() < deadline && server.exitCode === null) {
    ready = await request('/api/health/ready').then(result => result.status === 'ready', () => false)
    if (ready) break
    await delay(250)
  }
  assert(ready, 'isolated server did not become ready')

  const allowed = await request('/api/cameras', 'POST', { name: 'Allowed fixture', source: 'unused.mp4', type: 'file' })
  const hidden = await request('/api/cameras', 'POST', { name: 'Out of scope fixture', source: 'unused.mp4', type: 'file' })
  const pair = await request('/api/pairing', 'POST', {
    role: 'read_only', all_cameras: false, camera_ids: [allowed.id], expires_days: 1,
  })
  db = new DatabaseSync(dbPath)
  db.exec('PRAGMA busy_timeout=5000')
  const add = (camera, detail) => Number(db.prepare(
    "INSERT INTO events(ts,camera_id,type,level,track_id,detail,status) VALUES(datetime('now'),?,'enter',2,0,?,'new')"
  ).run(camera, detail).lastInsertRowid)
  const first = add(allowed.id, 'Initial event')
  const hiddenIds = [add(hidden.id, 'Hidden at startup')]
  browser = await chromium.launch({ args: ['--disable-gpu'] })
  const viewer = await browser.newContext()
  const claim = await viewer.request.post(origin + '/api/pairing/claim', {
    data: { code: pair.code, device_name: 'Replay acceptance', platform: 'web' },
  })
  assert(claim.ok(), 'paired browser fixture was rejected')
  const page = await viewer.newPage()
  await page.addInitScript(() => {
    window.replayTrace = []
    window.WebSocket = class extends window.WebSocket {
      constructor(...args) {
        super(...args)
        const url = new URL(this.url)
        if (url.pathname !== '/ws/events') return
        window.replaySocket = this
        window.replayTrace.push({ since: url.searchParams.get('since') })
        this.addEventListener('message', event => window.replayTrace.push(JSON.parse(event.data)))
      }
    }
  })
  let between
  await page.route(url => url.pathname === '/api/events', async route => {
    if (between !== undefined) return route.continue()
    const response = await route.fetch()
    between = add(allowed.id, 'Committed after initial HTTP result')
    await route.fulfill({ response })
  })
  await page.goto(origin + '/#/wall')
  const row = id => page.locator('[data-event-id="' + id + '"]')
  await row(first).waitFor({ timeout: 10_000 })
  await row(between).waitFor({ timeout: 10_000 })
  assert.equal((await page.evaluate(() => window.replayTrace))[0].since, String(first))

  async function disconnect() {
    await viewer.setOffline(true)
    await page.evaluate(() => window.replaySocket.close())
    await page.waitForFunction(() => window.replaySocket.readyState === WebSocket.CLOSED, null, { timeout: 6000 })
  }
  await disconnect()
  const later = [add(allowed.id, 'First disconnected event')]
  hiddenIds.push(add(hidden.id, 'Hidden during outage'))
  later.push(add(allowed.id, 'Second disconnected event'))
  await viewer.setOffline(false)
  for (const id of later) await row(id).waitFor({ timeout: 12_000 })

  await disconnect()
  const expired = later.at(-1)
  assert.equal(db.prepare('DELETE FROM events WHERE id=? AND camera_id=?').run(expired, allowed.id).changes, 1)
  const retained = add(allowed.id, 'Retained after expiry')
  hiddenIds.push(add(hidden.id, 'Hidden after expiry'))
  await viewer.setOffline(false)
  await row(retained).waitFor({ timeout: 12_000 })
  await page.getByText('部分历史事件已不可用，已刷新可查列表。', { exact: true }).waitFor({ timeout: 12_000 })
  assert.equal(await row(expired).count(), 0)
  const trace = await page.evaluate(() => window.replayTrace)
  const expected = [between, ...later, retained]
  assert.deepEqual(trace.filter(frame => Number.isSafeInteger(frame.id)).map(frame => frame.id), expected)
  assert(trace.some(frame => frame.stream_history_reset === true && frame.stream_cursor === expired))
  for (const id of hiddenIds) assert.equal(await row(id).count(), 0)
  const visible = await page.locator('[data-event-id]').evaluateAll(rows => rows.map(row => Number(row.dataset.eventId)))
  assert.deepEqual(visible, [retained, later[0], between, first])
  Object.assign(summary, { status: 'captured', initialCursor: first, replayed: expected, expired, finalRows: visible })
} catch (error) {
  summary.status = 'failed'
  summary.error = error.message
  throw error
} finally {
  db?.close()
  await browser?.close()
  if (server && server.exitCode === null && server.signalCode === null && exited) {
    try { await request('/api/system/shutdown', 'POST', {}) } catch { server.kill() }
    if (!await Promise.race([exited.then(() => true), delay(5000, false, { ref: false })])) server.kill()
    await exited
  }
  await writeFile(join(root, 'report.json'), JSON.stringify(summary, null, 2) + '\n')
  console.log(JSON.stringify(summary))
}
