// Run the MCP acceptance matrix on an isolated Windows package using the official client.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { once } from 'node:events'
import { access, copyFile, mkdir, mkdtemp, realpath, rm } from 'node:fs/promises'
import { createServer } from 'node:http'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import { setTimeout as delay } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'

const [packagePath, sdkPath] = process.argv.slice(2)
assert(packagePath && sdkPath, 'Usage: node web/scripts/smoke_mcp.mjs <package> <SDK install directory>')
const pkg = resolve(packagePath)
const executable = join(pkg, 'labsentinel-server.exe')
await access(executable)
const requireSdk = createRequire(join(resolve(sdkPath), 'package.json'))
const { Client } = requireSdk('@modelcontextprotocol/sdk/client/index.js')
const { StreamableHTTPClientTransport } = requireSdk('@modelcontextprotocol/sdk/client/streamableHttp.js')
const photo = fileURLToPath(new URL('../../Resources/acceptance/imgs/person-frame.jpg', import.meta.url))
const temp = await realpath(tmpdir())
const work = await mkdtemp(join(temp, 'labsentinel-mcp-'))
const data = join(work, 'data')
const dbPath = join(data, 'labsentinel.db')
const admin = randomBytes(32).toString('hex')
const clients = []
const verifiedWrites = []
let app, appExit, base

async function listen(server) {
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  return 'http://127.0.0.1:' + server.address().port
}

async function until(action, label, timeout = 25_000) {
  const end = Date.now() + timeout
  while (Date.now() < end) {
    const result = await action()
    if (result) return result
    await delay(150)
  }
  throw new Error(label + ' timed out')
}

async function adminCall(path, method = 'GET', body, expected = 200) {
  const response = await fetch(base + path, {
    method, signal: AbortSignal.timeout(8000),
    headers: { 'Content-Type': 'application/json', 'X-Admin-Token': admin },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  assert.equal(response.status, expected, method + ' ' + path)
  return response.status === 204 ? null : response.json()
}

async function rawRpc(headers = {}) {
  return fetch(base + '/mcp', {
    method: 'POST', signal: AbortSignal.timeout(5000),
    headers: { 'Content-Type': 'application/json', ...headers },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
  })
}

async function connect(token) {
  const client = new Client({ name: 'LabSentinel acceptance', version: '1.0.0' })
  clients.push(client)
  await client.connect(new StreamableHTTPClientTransport(new URL(base + '/mcp'), {
    requestInit: { headers: { Authorization: 'Bearer ' + token } },
  }))
  assert.equal(client.getServerVersion()?.name, 'LabSentinel')
  return client
}

async function checked(client, name, args = {}) {
  const result = await client.callTool({ name, arguments: args })
  assert(!result.isError, name)
  const record = result.structuredContent
  const receipt = record?._receipt
  assert.equal(receipt?.claims?.tool, name)
  assert.equal((await adminCall('/api/mcp/receipts/verify', 'POST', receipt)).valid, true, name)
  return record
}

async function denied(client, name, args = {}) {
  await assert.rejects(client.callTool({ name, arguments: args }), { code: -32003 }, name)
}

const policy = (patch) => adminCall('/api/mcp/config', 'PUT', patch)
const stopCamera = (id) => adminCall('/api/cameras/' + id + '/stop', 'POST', {})

function array(value, field) {
  assert(Array.isArray(value), field + ' must be an array')
  return value
}

async function toolNames(client) {
  return array((await client.listTools()).tools, 'tools').map(tool => tool.name).sort()
}

async function fixtureCamera(name) {
  const folder = join(work, name)
  await mkdir(folder)
  await copyFile(photo, join(folder, 'sample.jpg'))
  const { id } = await adminCall('/api/cameras', 'POST', { type: 'folder', name, source: folder, autostart: true })
  const event = await until(async () => {
    const checks = await adminCall('/api/events?category=check&camera_id=' + id)
    return array(checks, 'check events').find(row => row.type === 'photo_check')
  }, 'fixture event')
  await stopCamera(id)
  return { id, folder, event: event.id }
}

const mockCalls = []
const vision = createServer(async (request, response) => {
  const chunks = []
  for await (const chunk of request) chunks.push(chunk)
  try {
    const body = JSON.parse(Buffer.concat(chunks))
    const images = body.messages.flatMap(message => Array.isArray(message.content) ? message.content : [])
      .filter(part => part.type === 'image_url' && part.image_url?.url.startsWith('data:image/jpeg;base64,'))
    mockCalls.push({ path: request.url, images: images.length })
    response.writeHead(200, { 'Content-Type': 'application/json' })
    response.end(JSON.stringify({ choices: [{ message: { content: JSON.stringify({
      verdict: 'uncertain', reason: '单张验收图仅能确认画面有人。', confidence: 0.7,
    }) } }] }))
  } catch {
    response.writeHead(400)
    response.end()
  }
})

try {
  const visionBase = await listen(vision)
  const address = createServer()
  base = await listen(address)
  await new Promise(done => address.close(done))
  await mkdir(data)
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(LS_|ORT_)/i.test(key)))
  Object.assign(env, {
    LS_HOST: '127.0.0.1', LS_PORT: new URL(base).port, LS_DATA_DIR: data,
    LS_DB_PATH: dbPath, LS_INSTALL_DIR: pkg, LS_WEB_DIST: join(pkg, 'web'),
    LS_GPU: '0', LS_AUTH_READ_PROTECTED: '1', LS_ADMIN_TOKEN: admin,
    LS_LAUNCHER_NO_BROWSER: '1', LS_FFMPEG: join(pkg, 'tools', 'ffmpeg', 'ffmpeg.exe'),
    ORT_DYLIB_PATH: join(pkg, 'onnxruntime.dll'),
  })
  app = spawn(executable, [], { env, cwd: pkg, windowsHide: true, stdio: 'ignore' })
  appExit = once(app, 'exit')
  await until(async () => {
    if (app.exitCode !== null) throw new Error('LabSentinel exited during startup')
    try { return (await adminCall('/api/health/ready')).status === 'ready' } catch { return false }
  }, 'LabSentinel')
  assert.equal((await rawRpc()).status, 404)
  await policy({ enabled: true })
  assert.equal((await rawRpc()).status, 401)
  assert.equal((await rawRpc({ Authorization: 'Bearer lsmcp_' + 'X'.repeat(43) })).status, 401)
  assert.equal((await rawRpc({ Cookie: 'ls_session=fixture' })).status, 401)

  const camera = await fixtureCamera('allowed')
  const outside = await fixtureCamera('outside')
  const rule = await adminCall('/api/rules', 'POST', { camera_id: camera.id, behavior: 'no_mask', level: 2 })
  const issue = (scope) => adminCall('/api/mcp/tokens', 'POST', {
    name: 'isolated fixture ' + scope, scope, camera_ids: [camera.id], ttl_days: 1,
  }, 201)
  const read = await issue('read_only')
  const auth = { Authorization: 'Bearer ' + read.token }
  assert.equal((await rawRpc({ ...auth, Origin: 'https://other.example' })).status, 403)
  assert.equal((await rawRpc({ ...auth, Forwarded: 'for=192.0.2.1' })).status, 403)
  assert.equal((await rawRpc({ ...auth, Cookie: 'ls_session=fixture' })).status, 401)
  const reader = await connect(read.token)
  const eventArg = { event_id: camera.event }
  const cameraArg = { camera_id: camera.id }
  const reads = {
    get_system_status: {}, list_cameras: {}, get_camera_status: cameraArg,
    list_events: {}, get_event_evidence: eventArg, semantic_search_events: { q: 'person' },
    get_compliance_stats: {}, list_zones_and_rules: cameraArg, list_alert_silences: {},
    list_folder_images: cameraArg, list_skill_packs: {},
  }
  assert.deepEqual(await toolNames(reader), Object.keys(reads).sort())
  const readsSeen = {}
  for (const [name, args] of Object.entries(reads)) readsSeen[name] = await checked(reader, name, args)
  assert.deepEqual(array(readsSeen.list_cameras.cameras, 'cameras').map(row => row.id), [camera.id])
  assert.deepEqual(array(readsSeen.list_events.events, 'events').map(row => row.id), [camera.event])
  assert(array(readsSeen.list_folder_images.items, 'folder images').some(row => row.persons > 0))
  assert.equal(readsSeen.get_event_evidence.has_snapshot, true)
  assert.equal(readsSeen.semantic_search_events.status, 'disabled')
  await denied(reader, 'get_event_evidence', { event_id: outside.event })
  await denied(reader, 'get_event_evidence', { ...eventArg, include_thumbnail: true })
  for (const name of ['list_face_people', 'shutdown', 'delete_camera', 'update_mcp_policy']) {
    await denied(reader, name)
  }
  const tampered = structuredClone(readsSeen.get_event_evidence._receipt)
  tampered.claims.events[0].event_id = outside.event
  assert.equal((await adminCall('/api/mcp/receipts/verify', 'POST', tampered)).valid, false)
  await policy({ allow_image_content: true, confirm_high_risk: true })
  assert((await checked(reader, 'get_event_evidence', { ...eventArg, include_thumbnail: true })).thumbnail)
  await policy({ allow_image_content: false })

  const issued = await issue('policy')
  const writer = await connect(issued.token)
  const reason = 'Isolated acceptance fixture'
  const writes = {
    trigger_folder_scan: cameraArg,
    start_camera: cameraArg,
    stop_camera: { ...cameraArg, reason },
    toggle_rule: { rule_id: rule.id, enabled: false, reason },
    create_alert_silence: { scope: 'camera', ...cameraArg, duration_s: 120, reason },
    delete_alert_silence: { silence_id: 1 },
    add_event_note: { ...eventArg, text: reason },
    triage_event: { ...eventArg, status: 'acked', reason },
    trigger_event_vlm_review: eventArg,
  }
  for (const [name, args] of Object.entries(writes)) await denied(writer, name, args)
  await adminCall('/api/mcp/config', 'PUT', { allowed_write_tools: ['stop_camera'] }, 409)
  async function grantAndCall(name, args = writes[name]) {
    await policy({ allowed_write_tools: [name], confirm_high_risk: true })
    assert.deepEqual(await toolNames(writer), [...Object.keys(reads), name].sort())
    const record = await checked(writer, name, args)
    verifiedWrites.push(name)
    return record
  }

  await copyFile(photo, join(camera.folder, 'second.jpg'))
  assert.equal((await grantAndCall('trigger_folder_scan')).started, true)
  await until(async () => {
    const records = await checked(reader, 'list_folder_images', cameraArg)
    return array(records.items, 'folder images').length === 2
  }, 'folder scan')
  await stopCamera(camera.id)
  assert.equal((await grantAndCall('start_camera')).started, true)
  await until(async () => (await adminCall('/api/cameras/' + camera.id + '/status')).status === 'running', 'camera start')
  assert.equal((await grantAndCall('stop_camera')).was_running, true)
  assert.equal((await adminCall('/api/cameras/' + camera.id + '/status')).status, 'stopped')
  assert.equal((await grantAndCall('toggle_rule')).enabled, false)
  const listed = await checked(reader, 'list_zones_and_rules', cameraArg)
  assert.equal(array(listed.rules, 'rules').find(row => row.id === rule.id)?.enabled, false)
  const silence = await grantAndCall('create_alert_silence')
  const hasSilence = async () => {
    const result = await checked(reader, 'list_alert_silences')
    return array(result.items, 'silences').some(row => row.id === silence.id)
  }
  assert(await hasSilence())
  await grantAndCall('delete_alert_silence', { silence_id: silence.id })
  assert(!await hasSilence())
  assert((await grantAndCall('add_event_note')).note_id > 0)
  await denied(reader, 'add_event_note', writes.add_event_note)
  assert.equal((await grantAndCall('triage_event')).status, 'acked')
  const notes = await adminCall('/api/events/' + camera.event + '/notes')
  assert.equal(array(notes, 'notes').filter(note => note.text === reason).length, 2)
  await adminCall('/api/detect/config', 'PUT', {
    mode: 'cascade', vlm: { provider: 'openai_compat', url: visionBase + '/v1', model: 'fixture', timeout_s: 5 },
  })
  const reviewed = await grantAndCall('trigger_event_vlm_review')
  assert.equal(reviewed.verdict, 'uncertain')
  assert.deepEqual(mockCalls, [{ path: '/v1/chat/completions', images: 1 }])
  const updated = array(await adminCall('/api/events?status=acked&camera_id=' + camera.id), 'triaged events')
    .find(event => event.id === camera.event)
  assert.equal(updated?.status, 'acked')
  assert.equal(updated.ai_verdict, 'uncertain')

  const database = new DatabaseSync(dbPath, { readOnly: true })
  try {
    const rows = database.prepare('SELECT action, detail FROM audit_log WHERE actor = ?').all('mcp:' + issued.id)
    for (const name of Object.keys(writes)) {
      const decisions = rows.filter(row => row.action === 'mcp:' + name).map(row => JSON.parse(row.detail).decision)
      assert.deepEqual(decisions.sort(), ['allowed', 'denied'], name)
    }
  } finally {
    database.close()
  }
  await adminCall('/api/mcp/tokens/' + issued.id, 'DELETE', undefined, 204)
  assert.equal((await rawRpc({ Authorization: 'Bearer ' + issued.token })).status, 401)
  console.log(JSON.stringify({ sdk: 'official TypeScript SDK', reads: Object.keys(reads).length,
    writes: verifiedWrites.length, audit: 'allowed and denied for each write', visionRequests: mockCalls.length }))
} finally {
  await Promise.allSettled(clients.map(client => client.close()))
  if (app && app.exitCode === null && app.signalCode === null && app.pid) {
    try { await adminCall('/api/system/shutdown', 'POST', {}) } catch { app.kill() }
    if (!await Promise.race([appExit.then(() => true), delay(5000, false, { ref: false })])) app.kill()
  }
  if (appExit) await appExit
  await new Promise(done => vision.close(done))
  assert.equal(dirname(await realpath(work)), temp)
  await rm(work, { recursive: true, force: true })
}
