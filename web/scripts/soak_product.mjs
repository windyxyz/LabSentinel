// Run an isolated Windows package with loopback RTSP, a real wall page and controlled outages.
import assert from 'node:assert/strict'
import { execFile, spawn } from 'node:child_process'
import { randomBytes } from 'node:crypto'
import { once } from 'node:events'
import { access, appendFile, copyFile, cp, mkdir, open, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { basename, join, resolve } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import { setTimeout as delay } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'
import { parseArgs, promisify } from 'node:util'
import { chromium } from 'playwright'

const { values } = parseArgs({ options: {
  package: { type: 'string' }, out: { type: 'string' }, backend: { type: 'string', default: 'cpu' },
  'gpu-id': { type: 'string' },
  cameras: { type: 'string', default: '1' }, seconds: { type: 'string' }, warmup: { type: 'string', default: '0' },
  sample: { type: 'string', default: '10' }, 'outage-every': { type: 'string', default: '0' },
  'outage-seconds': { type: 'string', default: '15' },
  'vlm-runtime': { type: 'string' }, 'vlm-model': { type: 'string' }, 'vlm-projector': { type: 'string' },
  'vlm-every': { type: 'string', default: '120' }, 'vlm-gpu-layers': { type: 'string', default: '0' },
  'vlm-device': { type: 'string' }, 'vlm-vram-budget-mb': { type: 'string' },
  'cv-vram-reserve-mb': { type: 'string' },
  'latency-hud': { type: 'boolean', default: false },
} })
assert(values.package && values.out && values.seconds, 'Specify --package, --out and --seconds')
assert(['cpu', 'dml', 'auto_vulkan'].includes(values.backend), 'Invalid backend')
function seconds(key, minimum) {
  const value = Number(values[key])
  assert(Number.isSafeInteger(value) && value >= minimum, 'Invalid --' + key)
  return value
}
const cameras = seconds('cameras', 1)
assert([1, 4, 9].includes(cameras), 'Use 1, 4 or 9 cameras')
const duration = seconds('seconds', 1) * 1000
const warmup = seconds('warmup', 0) * 1000
const interval = seconds('sample', 1) * 1000
const outageEvery = seconds('outage-every', 0) * 1000
const outageMs = seconds('outage-seconds', 1) * 1000
assert(!outageEvery || outageEvery > outageMs, 'Outages must be spaced farther apart than their duration')
const vlmSources = [values['vlm-runtime'], values['vlm-model'], values['vlm-projector']]
const withVision = vlmSources.some(Boolean)
assert(!withVision || vlmSources.every(Boolean), 'Specify the VLM runtime, model and projector together')
const vlmEvery = seconds('vlm-every', 1) * 1000
const vlmLayers = seconds('vlm-gpu-layers', 0)
assert(vlmLayers <= 128, 'VLM GPU layers must be 0..128')
const vlmGpu = {}
if (values['vlm-device'] !== undefined) {
  assert(/^(auto|Vulkan(?:0|[1-9]\d*))$/.test(values['vlm-device']), 'Invalid --vlm-device')
  vlmGpu.vulkan_device = values['vlm-device']
}
for (const [arg, key] of [['vlm-vram-budget-mb', 'vram_budget_mb'], ['cv-vram-reserve-mb', 'cv_vram_reserve_mb']]) {
  if (values[arg] === undefined) continue
  vlmGpu[key] = seconds(arg, 0)
  assert(vlmGpu[key] <= 1048576, 'Invalid --' + arg)
}
const pkg = resolve(values.package)
const output = resolve(values.out)
const data = join(output, 'data')
const install = withVision ? join(output, 'install') : pkg
const ffmpeg = join(pkg, 'tools', 'ffmpeg', 'ffmpeg.exe')
const go2rtc = join(pkg, 'tools', 'go2rtc.exe')
const binary = join(pkg, 'labsentinel-server.exe')
const dbPath = join(data, 'labsentinel.db')
const reportPath = join(output, 'report.json')
const admin = randomBytes(32).toString('hex')
const ids = []
const ownedMediaPids = new Set()
const exec = promisify(execFile)
let server, producer, browser, page, cdp, origin, wallFailure, startedAt, visionJob, visionFailure
let interrupted = false
const report = { status: 'running', backend: values.backend, cameras, durationMs: duration,
  warmupMs: warmup, samples: 0, outages: [] }
if (values['gpu-id']) report.requestedGpu = values['gpu-id']
process.on('SIGINT', () => { interrupted = true })

function array(value, field) {
  assert(Array.isArray(value), field + ' must be an array')
  return value
}

async function reservePorts(count) {
  const sockets = Array.from({ length: count }, () => createServer())
  await Promise.all(sockets.map(async socket => {
    socket.listen(0, '127.0.0.1')
    await once(socket, 'listening')
  }))
  const ports = sockets.map(socket => socket.address().port)
  await Promise.all(sockets.map(socket => new Promise(done => socket.close(done))))
  return ports
}

async function logChild(program, args, file, env = process.env) {
  const log = await open(join(output, file), 'a')
  try {
    const child = spawn(program, args, { cwd: output, env, windowsHide: true, stdio: ['ignore', log.fd, log.fd] })
    await once(child, 'spawn')
    return child
  } finally {
    await log.close()
  }
}

async function stop(child, tree = false) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  const exited = once(child, 'exit')
  if (tree) {
    // The fixture producer owns FFmpeg children outside the product's Windows job.
    await exec('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true, timeout: 5000 })
  } else {
    child.kill()
  }
  await exited
}

async function checkCancelled() {
  const requested = await access(join(output, 'stop.requested')).then(() => true, () => false)
  if (interrupted || requested) throw new Error('measurement cancelled')
}

async function until(check, label, timeout = 45_000) {
  const deadline = performance.now() + timeout
  while (performance.now() < deadline) {
    await checkCancelled()
    const value = await check()
    if (value) return value
    await delay(250)
  }
  throw new Error(label + ' timed out')
}

async function app(path, method = 'GET', json, timeout = 6000) {
  const reply = await fetch(origin + path, {
    method, signal: AbortSignal.timeout(timeout),
    headers: { 'Content-Type': 'application/json', 'X-Admin-Token': admin },
    body: json === undefined ? undefined : JSON.stringify(json),
  })
  assert(reply.ok, method + ' ' + path + ' returned ' + reply.status)
  return reply.json()
}

async function powershell(script) {
  const { stdout } = await exec('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script],
    { windowsHide: true, timeout: 20_000 })
  return JSON.parse(stdout)
}

async function processSample() {
  const chrome = array((await cdp.send('SystemInfo.getProcessInfo')).processInfo, 'Chromium process info')
  const roots = [server.pid, producer.pid]
  const all = [...new Set([...roots, ...chrome.map(row => row.id)])]
  assert(all.every(Number.isSafeInteger))
  const data = await powershell(`
    $parents = @(${roots.join(',')})
    $media = @(Get-CimInstance Win32_Process -Filter "Name='go2rtc.exe' OR Name='ffmpeg.exe' OR Name='llama-server.exe'" -ErrorAction Stop)
    do {
      $count = $parents.Count
      $parents += @($media | Where-Object { $parents -contains $_.ParentProcessId -and $parents -notcontains $_.ProcessId } | ForEach-Object ProcessId)
    } until ($parents.Count -eq $count)
    $watched = @(${all.join(',')}) + $parents
    $processes = @(Get-Process -Id ($watched | Sort-Object -Unique) -ErrorAction SilentlyContinue |
      Select-Object Id,ProcessName,WorkingSet64,HandleCount,@{n='Started';e={$_.StartTime.ToUniversalTime().Ticks.ToString()}})
    $graphics = @(Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUProcessMemory -Filter "Name LIKE 'pid[_]${server.pid}[_]%'" -ErrorAction SilentlyContinue |
      Select-Object Name,DedicatedUsage,SharedUsage)
    ConvertTo-Json -Compress -InputObject @{ processes = $processes; graphics = $graphics }
  `)
  const chromeIds = new Set(chrome.map(proc => proc.id))
  const rows = array(data.processes, 'Windows process sample')
  const product = rows.filter(row => !chromeIds.has(row.Id))
  product.forEach(row => { if (row.Id !== server.pid) ownedMediaPids.add(row.Id + ':' + row.Started) })
  assert(product.some(row => row.Id === server.pid), 'product process missing')
  const browsers = rows.filter(row => chromeIds.has(row.Id))
  assert(browsers.length, 'Chromium process sample missing')
  return { product, browsers, graphics: array(data.graphics, 'Windows GPU memory sample') }
}

async function windowsEvents(from, until) {
  assert([from, until].every(Number.isSafeInteger))
  return powershell(`
    $from = [DateTimeOffset]::FromUnixTimeMilliseconds(${from}).LocalDateTime
    $until = [DateTimeOffset]::FromUnixTimeMilliseconds(${until}).LocalDateTime
    try {
      $rows = @(Get-WinEvent -FilterHashtable @{LogName='System'; StartTime=$from; EndTime=$until; Id=@(41,4101)} -ErrorAction Stop)
    } catch {
      if ($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*') { throw }
      $rows = @()
    }
    ConvertTo-Json -Compress -InputObject @($rows | Where-Object {
      $_.ProviderName -in @('Microsoft-Windows-Kernel-Power','Display')
    } | Select-Object Id,ProviderName,RecordId)
  `)
}

async function wallSample() {
  if (wallFailure) throw new Error('Wall browser stopped: ' + wallFailure)
  return page.evaluate(() => ({
    tiles: document.querySelectorAll('.camera-preview').length,
    decodedStreams: Array.from(document.querySelectorAll('.camera-preview video'))
      .filter(video => video.videoWidth > 0 && !video.paused && video.currentTime > 0).length,
    eventsShown: document.querySelectorAll('.wall aside li').length,
  }))
}

async function movingWall() {
  const frames = () => page.evaluate(() => Array.from(document.querySelectorAll('.camera-preview video'))
    .map(video => video.getVideoPlaybackQuality().totalVideoFrames))
  const before = await frames()
  await delay(600)
  return (await frames()).some((count, index) => count > (before[index] ?? 0))
}

const target = { cpu: 'CPU', dml: 'DirectML(GPU)', auto_vulkan: 'Vulkan(GPU)' }[values.backend]
async function measuredCameras() {
  assert(server.exitCode === null && server.signalCode === null, 'product process exited')
  const system = await app('/api/system')
  const analysis = array(system.perf?.analysis, 'analysis metrics')
  return ids.map(id => analysis.find(row => row.camera_id === id) ?? null)
}

async function writeSample() {
  const at = Date.now()
  const analysis = await measuredCameras()
  assert(analysis.every(Boolean), 'analysis source missing from performance report')
  const runtime = (await app('/api/detect/config')).runtime
  const devices = array(runtime?.camera_devices, 'model devices')
  assert(ids.every(id => devices.some(row => row.camera_id === id && row.person === target && row.ppe === target)),
    'a camera is not using its requested person and PPE backend')
  const wall = await wallSample()
  assert.equal(wall.tiles, cameras)
  assert(wall.decodedStreams <= 5, 'wall exceeded five decoded live streams')
  const resources = await processSample()
  await appendFile(join(output, 'samples.jsonl'), JSON.stringify({ at, analysis, devices, wall, resources }) + '\n')
  report.samples++
  report.maxDecodedStreams = Math.max(report.maxDecodedStreams ?? 0, wall.decodedStreams)
}

async function checkEventClip() {
  const events = array(await app('/api/events?limit=100'), 'events')
  for (const event of events) {
    if (!ids.includes(event.camera_id) || event.has_video !== true) continue
    const part = await fetch(origin + '/api/events/' + event.id + '/video', {
      headers: { 'X-Admin-Token': admin, Range: 'bytes=0-31' }, signal: AbortSignal.timeout(4000),
    })
    const bytes = Buffer.from(await part.arrayBuffer())
    if (part.status === 206 && part.headers.get('content-range')?.startsWith('bytes 0-31/') &&
      bytes.byteLength === 32 && bytes.toString('ascii', 4, 8) === 'ftyp') return event.id
  }
  return null
}

async function checkClipPlayback(event) {
  const context = await browser.newContext({ extraHTTPHeaders: { 'X-Admin-Token': admin } })
  try {
    const playback = await context.newPage()
    const testPage = origin + '/soak-clip'
    await context.route(testPage, route => route.fulfill({
      contentType: 'text/html',
      body: `<video muted autoplay src="/api/events/${event}/video"></video>`,
    }))
    await playback.goto(testPage)
    await playback.waitForFunction(() => {
      const video = document.querySelector('video')
      return video?.videoWidth > 0 && video.getVideoPlaybackQuality().totalVideoFrames > 1
    }, null, { timeout: 12_000 })
    assert(await movingWall(), 'wall stopped decoding during evidence playback')
  } finally {
    await context.close()
  }
}

async function askVisualQuestion() {
  const events = array(await app('/api/events?limit=30&camera_id=' + ids[0]), 'events with evidence')
  const event = events.find(row => row.has_snapshot === true)
  assert(event, 'no event with a snapshot is available for visual inference')
  const session = await app('/api/assistant/sessions', 'POST', { title: 'Soak fixture', scope: { camera_id: ids[0] } })
  const before = await measuredCameras()
  const requestAt = Date.now()
  const message = await app('/api/assistant/sessions/' + session.id + '/messages', 'POST', {
    request_id: 'soak-' + randomBytes(8).toString('hex'), revision: 0, include_images: true,
    question: '复核事件 #' + event.id,
  }, 40_000)
  assert.equal(message.evidence_mode, 'images', 'local VLM did not receive visual evidence')
  assert(message.model && message.answer, 'local VLM did not return an answer')
  assert.deepEqual(array(message.facts?.image_event_ids, 'visual evidence IDs'), [event.id])
  const after = await measuredCameras()
  await appendFile(join(output, 'vision.jsonl'), JSON.stringify({
    requestAt, endedAt: Date.now(), event: event.id, model: message.model,
    cvFrames: ids.map((id, index) => [id, after[index].inferred - before[index].inferred]),
  }) + '\n')
  report.vision.requests++
}

await Promise.all([binary, ffmpeg, go2rtc].map(file => access(file)))
await mkdir(output)
await mkdir(data)
try {
  if (withVision) {
    await cp(pkg, install, { recursive: true, errorOnExist: true, force: false })
    await cp(resolve(vlmSources[0]), join(install, 'tools', 'llama'), { recursive: true, errorOnExist: true, force: false })
    const modelRoot = join(data, 'models', 'local-vlm')
    await mkdir(modelRoot, { recursive: true })
    await Promise.all(vlmSources.slice(1).map(path => copyFile(resolve(path), join(modelRoot, basename(path)))))
  }
  const [appPort, sourceApi, sourceRtsp, mediaApi, mediaRtsp, mediaRtc] = await reservePorts(6)
  origin = 'http://127.0.0.1:' + appPort
  const photo = fileURLToPath(new URL('../../Resources/acceptance/imgs/person-frame.jpg', import.meta.url))
  await exec(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-nostdin',
    '-f', 'lavfi', '-i', 'color=c=gray:s=1920x1080:r=12:d=5',
    '-loop', '1', '-framerate', '12', '-i', photo, '-filter_complex_threads', '1',
    '-filter_complex', '[1:v]scale=1920:1080:force_original_aspect_ratio=decrease,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,setsar=1,trim=duration=6[p];[0:v][p]concat=n=2:v=1:a=0[out]',
    '-map', '[out]', '-t', '11', '-an', '-c:v', 'libx264', '-preset', 'ultrafast', '-g', '12',
    '-threads', '2', '-pix_fmt', 'yuv420p', join(output, 'fixture.mp4')],
  { windowsHide: true, timeout: 40_000 })
  const streamNames = Array.from({ length: cameras }, (_, index) => 'fixture-' + (index + 1))
  // go2rtc keys its FFmpeg RTSP callback by command. Each producer needs a unique input path.
  await Promise.all(streamNames.map(name => copyFile(join(output, 'fixture.mp4'), join(output, name + '.mp4'))))
  const sourceConfig = JSON.stringify({
    api: { listen: '127.0.0.1:' + sourceApi }, rtsp: { listen: '127.0.0.1:' + sourceRtsp },
    webrtc: { listen: '' }, ffmpeg: { bin: ffmpeg, file: '-re -stream_loop -1 -i {input}' },
    streams: Object.fromEntries(streamNames.map(name => [name, 'ffmpeg:' + name + '.mp4#video=copy'])),
  })
  async function startSource() {
    producer = await logChild(go2rtc, ['-c', sourceConfig], 'source.log')
    await until(async () => {
      if (producer.exitCode !== null) throw new Error('RTSP fixture exited')
      try { return (await fetch('http://127.0.0.1:' + sourceApi + '/api/streams', { signal: AbortSignal.timeout(1000) })).ok } catch { return false }
    }, 'RTSP fixture')
  }
  await startSource()
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(LS_|ORT_)/i.test(key)))
  Object.assign(env, {
    LS_HOST: '127.0.0.1', LS_PORT: String(appPort), LS_DATA_DIR: data, LS_DB_PATH: dbPath,
    LS_INSTALL_DIR: install, LS_WEB_DIST: join(install, 'web'), LS_AUTH_READ_PROTECTED: '1',
    LS_ADMIN_TOKEN: admin, LS_LAUNCHER_NO_BROWSER: '1', LS_ANALYSIS_FPS: '5', LS_FFMPEG: ffmpeg,
    ORT_DYLIB_PATH: join(pkg, 'onnxruntime.dll'), LS_GO2RTC: go2rtc, LS_GO2RTC_PORT: String(mediaApi),
    LS_GO2RTC_RTSP_LISTEN: '127.0.0.1:' + mediaRtsp, LS_GO2RTC_WEBRTC_LISTEN: ':' + mediaRtc,
  })
  server = await logChild(join(install, basename(binary)), [], 'product.log', env)
  await until(async () => {
    if (server.exitCode !== null) throw new Error('product exited during startup')
    try { return (await app('/api/health/ready')).status === 'ready' } catch { return false }
  }, 'product readiness')
  await app('/api/detect/config', 'PUT', {
    mode: 'local',
    runtime: { backend: values.backend, ...(values['gpu-id'] && { gpu_id: values['gpu-id'] }) },
    perf: { motion_gate_enabled: false },
    cascade: { daily_budget: 0 },
  })
  const cameraSetupAt = performance.now()
  for (const name of streamNames) {
    const source = 'rtsp://127.0.0.1:' + sourceRtsp + '/' + name
    const camera = await app('/api/cameras', 'POST', { name, type: 'rtsp', source })
    ids.push(camera.id)
    const zone = await app('/api/zones', 'POST', { camera_id: camera.id, name: 'Fixture', polygon: [[0, 0], [1279, 0], [1279, 719], [0, 719]] })
    await app('/api/rules', 'POST', { camera_id: camera.id, zone_id: zone.id, behavior: 'enter', cooldown_s: 2, level: 2 })
    if (cameras === 9) await app('/api/cameras/' + camera.id + '/analysis-source', 'PUT', { mode: 'substream', source })
    await app('/api/cameras/' + camera.id + '/start', 'POST', {})
  }
  browser = await chromium.launch({ args: ['--autoplay-policy=no-user-gesture-required', '--disable-gpu'] })
  cdp = await browser.newBrowserCDPSession()
  page = await browser.newPage({ viewport: { width: 1600, height: 1000 } })
  page.on('crash', () => { wallFailure = 'page crashed' })
  browser.on('disconnected', () => { wallFailure = 'browser disconnected' })
  await page.addInitScript(token => {
    localStorage.setItem('admin_token', token)
    localStorage.setItem('ls-welcome-done', '1')
  }, admin)
  await page.goto(origin + '/#/wall')
  await page.getByLabel('宫格切换').selectOption(String(cameras))
  await until(async () => (await measuredCameras()).every(row => row?.inferred > 0), 'camera warmup')
  report.cameraSetupToAnalysisMs = Math.round(performance.now() - cameraSetupAt)
  await until(movingWall, 'wall video')
  if (withVision) {
    const vlm = await app('/api/detect/local-vlm/start', 'POST', {
      model: basename(vlmSources[1]), projector: basename(vlmSources[2]), gpu_layers: vlmLayers,
      ...vlmGpu,
    }, 105_000)
    report.vision = {
      startupStatus: vlm.status, gpu: vlm.gpu, fallbackReason: vlm.fallback_reason,
      intervalMs: vlmEvery, requests: 0,
    }
    assert.equal(vlm.status, vlmLayers > 0 ? 'gpu_requested' : 'cpu', 'unexpected local VLM startup mode')
    if (vlmLayers > 0 && vlmGpu.vulkan_device && vlmGpu.vulkan_device !== 'auto') {
      assert.equal(vlm.gpu?.device.id, vlmGpu.vulkan_device, 'unexpected local VLM device')
    }
    await app('/api/assistant/roles', 'PUT', { chat: vlm.profile_id })
  }
  const warmedAt = performance.now()
  while (performance.now() - warmedAt < warmup) {
    await checkCancelled()
    assert(server.exitCode === null && producer.exitCode === null && !wallFailure, 'fixture stopped during warmup')
    const remaining = warmup - (performance.now() - warmedAt)
    if (remaining > 0) await delay(Math.min(1000, remaining))
  }

  startedAt = Date.now()
  report.startedAt = startedAt
  const start = performance.now()
  const finish = start + duration
  let nextSample = start
  let nextOutage = outageEvery ? start + outageEvery : Infinity
  let nextVision = withVision ? start + vlmEvery : Infinity
  while (performance.now() < finish) {
    await checkCancelled()
    if (visionFailure) throw visionFailure
    const now = performance.now()
    if (now >= nextOutage && nextOutage + outageMs < finish) {
      const before = await measuredCameras()
      assert(before.every(Boolean), 'camera did not report before scheduled outage')
      await stop(producer, true)
      const disconnectedAt = Date.now()
      const disconnectedUntil = performance.now() + outageMs
      await until(() => performance.now() >= disconnectedUntil, 'outage duration', outageMs + 2000)
      await startSource()
      const sourceReadyAt = Date.now()
      const disconnected = await measuredCameras()
      await until(async () => (await measuredCameras()).every((row, index) =>
        row?.inferred > (disconnected[index]?.inferred ?? before[index].inferred)), 'cameras recover from outage', 60_000)
      const cameraRecoveredAt = Date.now()
      await until(movingWall, 'wall recovers from outage')
      report.outages.push({ disconnectedAt, sourceReadyAt, cameraRecoveredAt, wallRecoveredAt: Date.now() })
      nextOutage += outageEvery
    }
    if (now >= nextVision && nextVision < finish && !visionJob) {
      visionJob = askVisualQuestion().catch(error => { visionFailure = error }).finally(() => { visionJob = null })
      nextVision += vlmEvery
    }
    if (now >= nextSample) {
      await writeSample()
      nextSample += interval * (Math.floor((now - nextSample) / interval) + 1)
    }
    await delay(100)
  }
  await visionJob
  if (visionFailure) throw visionFailure
  if (withVision) {
    assert.equal(report.vision.requests, Math.floor((duration - 1) / vlmEvery), 'scheduled visual calls did not all complete')
  }
  await writeSample()
  report.elapsedMs = Math.round(performance.now() - start)
  report.eventClipId = await checkEventClip()
  assert.notEqual(report.eventClipId, null, 'no readable event clip was produced')
  await checkClipPlayback(report.eventClipId)
  report.eventClipPlayback = true
  if (values['latency-hud']) {
    await page.bringToFront()
    await page.getByRole('button', { name: '分析耗时' }).click()
    const hud = page.getByRole('region', { name: '分析耗时' })
    await hud.getByRole('columnheader', { name: 'Vulkan 首次推理' }).waitFor({ state: 'visible' })
    const queueColumn = await hud.getByRole('columnheader', { name: 'GPU 调度等待' }).evaluate(cell => cell.cellIndex)
    await until(async () => {
      const queues = await hud.locator('tbody tr').evaluateAll((rows, column) =>
        rows.map(row => row.cells[column]?.textContent.trim()), queueColumn)
      return queues.length === cameras && queues.every(queue => /^\d+\.\d$/.test(queue))
    }, 'GPU queue values shown in latency panel')
    report.latencyHud = true
  }
  report.graphicsEvents = await windowsEvents(startedAt, Date.now())
  assert.equal(array(report.graphicsEvents, 'graphics events').length, 0, 'Windows graphics reset or unexpected power loss')
  const expectedOutages = outageEvery ? Math.max(0, Math.floor((duration - outageMs - 1) / outageEvery)) : 0
  assert.equal(report.outages.length, expectedOutages, 'scheduled outages were not all exercised')
  report.status = 'captured'
} catch (error) {
  report.status = interrupted || error.message === 'measurement cancelled' ? 'cancelled' : 'failed'
  report.error = error.message
  throw error
} finally {
  try {
    if (browser) await browser.close()
    if (server && server.exitCode === null && server.signalCode === null) {
      const exited = once(server, 'exit')
      try { await app('/api/system/shutdown', 'POST', {}) } catch { /* stop only the isolated child */ }
      if (await Promise.race([exited.then(() => true), delay(5000, false, { ref: false })])) {
        report.gracefulExit = server.exitCode === 0
      }
    }
    await stop(server)
    await stop(producer, true)
    if (ownedMediaPids.size) {
      const observed = [...ownedMediaPids]
      const numbers = [...new Set(observed.map(value => Number(value.split(':')[0])))]
      const alive = await powershell(`ConvertTo-Json -Compress -InputObject @(Get-Process -Id ${numbers.join(',')} -ErrorAction SilentlyContinue | ForEach-Object { [string]$_.Id + ':' + $_.StartTime.ToUniversalTime().Ticks.ToString() })`)
      report.remainingObservedMedia = array(alive, 'remaining media').filter(value => observed.includes(value))
    }
    const database = new DatabaseSync(dbPath, { readOnly: true })
    try { report.database = database.prepare('PRAGMA integrity_check').all() } finally { database.close() }
    if (report.status === 'captured') {
      assert.equal(report.gracefulExit, true, 'product did not exit gracefully')
      assert.deepEqual(report.remainingObservedMedia, [], 'a sampled media child remains after shutdown')
      assert.deepEqual(report.database.map(row => row.integrity_check), ['ok'], 'SQLite integrity check failed')
    }
  } catch (error) {
    report.cleanupError = error.message
    if (report.status === 'captured') report.status = 'failed'
    process.exitCode = 1
  } finally {
    await writeFile(reportPath, JSON.stringify(report, null, 2) + '\n')
    console.log(reportPath)
  }
}
