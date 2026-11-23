// Exercises the shipped go2rtc protocol, fallback, recovery and share revocation in Chromium.
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { once } from 'node:events'
import { mkdtempSync, rmSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { chromium } from 'playwright'

const pkg = resolve(process.argv[2] || '../Resources/dist/rust/labsentinel-server')
const ffmpeg = join(pkg, 'tools', 'ffmpeg', 'ffmpeg.exe')
const go2rtc = join(pkg, 'tools', 'go2rtc.exe')
const work = mkdtempSync(join(tmpdir(), 'labsentinel-playback-'))
const adminToken = 'playback-test-admin-local'
const logs = []

async function freePort() {
  const socket = createServer()
  socket.listen(0, '127.0.0.1')
  await once(socket, 'listening')
  const { port } = socket.address()
  await new Promise(done => socket.close(done))
  return port
}

function silentChild(program, args, env = process.env, cwd = work) {
  const child = spawn(program, args, { cwd, env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
  for (const stream of [child.stdout, child.stderr]) {
    stream.on('data', data => {
      logs.push(data.toString().trim())
      if (logs.length > 50) logs.shift()
    })
  }
  return child
}

async function until(action, name, timeout = 18000) {
  const end = Date.now() + timeout
  while (Date.now() < end) {
    try { if (await action()) return } catch { /* transient startup or reconnect */ }
    await delay(150)
  }
  throw new Error(name + ' timed out')
}

async function stop(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null || !child.pid) return
  const exit = once(child, 'exit')
  child.kill()
  await exit
}

const ports = await Promise.all(Array.from({ length: 6 }, freePort))
const [sourceApi, sourceRtsp, appPort, mediaApi, mediaRtsp, mediaRtc] = ports
const origin = 'http://127.0.0.1:' + appPort

async function call(path, method = 'GET', value) {
  const res = await fetch(origin + path, {
    method,
    headers: { 'Content-Type': 'application/json', 'X-Admin-Token': adminToken },
    body: value === undefined ? undefined : JSON.stringify(value),
    signal: AbortSignal.timeout(5000),
  })
  if (!res.ok) throw new Error(method + ' ' + path + ' returned ' + res.status + ': ' + await res.text())
  return res.json()
}

let producer
let server
let browser
try {
  const fixture = spawnSync(ffmpeg, [
    '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc2=s=640x360:r=12',
    '-t', '5', '-an', '-pix_fmt', 'yuv420p', '-c:v', 'libx264', '-profile:v', 'baseline',
    '-g', '12', '-preset', 'ultrafast', 'fixture.mp4',
  ], { cwd: work, windowsHide: true, encoding: 'utf8' })
  assert.equal(fixture.status, 0, fixture.stderr)
  producer = silentChild(go2rtc, ['-c', JSON.stringify({
    api: { listen: '127.0.0.1:' + sourceApi },
    rtsp: { listen: '127.0.0.1:' + sourceRtsp },
    webrtc: { listen: '' },
    ffmpeg: { bin: ffmpeg, file: '-re -stream_loop -1 -i {input}' },
    streams: { fixture: 'ffmpeg:fixture.mp4#video=copy' },
  })])
  await until(async () => (await fetch('http://127.0.0.1:' + sourceApi + '/api/streams')).ok, 'source gateway')
  const rtsp = 'rtsp://127.0.0.1:' + sourceRtsp + '/fixture'
  const decoded = spawnSync(ffmpeg, [
    '-hide_banner', '-loglevel', 'error', '-rtsp_transport', 'tcp', '-timeout', '10000000',
    '-i', rtsp, '-an', '-sn', '-dn', '-vf', 'fps=5,scale=1280:720',
    '-frames:v', '1', '-pix_fmt', 'bgr24', '-f', 'rawvideo', 'pipe:1',
  ], { windowsHide: true, timeout: 18000, maxBuffer: 4 * 1024 * 1024 })
  assert.equal(decoded.status, 0, String(decoded.stderr || decoded.error || 'failed to decode RTSP fixture'))
  assert.equal(decoded.stdout.length, 1280 * 720 * 3)

  const env = {
    ...process.env, LS_HOST: '127.0.0.1', LS_PORT: String(appPort),
    LS_DATA_DIR: join(work, 'data'), LS_DB_PATH: join(work, 'data', 'labsentinel.db'),
    LS_INSTALL_DIR: pkg, LS_WEB_DIST: join(pkg, 'web'), LS_ADMIN_TOKEN: adminToken, LS_AUTH_READ_PROTECTED: '1',
    LS_GPU: '0', LS_FFMPEG: ffmpeg, ORT_DYLIB_PATH: join(pkg, 'onnxruntime.dll'),
    LS_GO2RTC: go2rtc, LS_GO2RTC_PORT: String(mediaApi),
    LS_GO2RTC_RTSP_LISTEN: '127.0.0.1:' + mediaRtsp,
    LS_GO2RTC_WEBRTC_LISTEN: ':' + mediaRtc,
  }
  delete env.LS_GO2RTC_ADDR
  server = silentChild(join(pkg, 'labsentinel-server.exe'), [], env, pkg)
  await until(async () => (await call('/api/health/ready')).status === 'ready', 'LabSentinel')
  await call('/api/detect/config', 'PUT', { runtime: { ep: 'cpu' } })
  const camera = await call('/api/cameras', 'POST', {
    name: 'Playback fixture', type: 'rtsp', source: rtsp, autostart: true,
  })
  const cameraPath = '/api/cameras/' + camera.id
  await until(async () => (await call(cameraPath + '/webrtc')).mode === 'webrtc', 'camera registration')
  const analysisSource = (mode) => until(async () => {
    const system = await call('/api/system')
    return Array.isArray(system.perf?.analysis) && system.perf.analysis.some(
      level => level.camera_id === camera.id && level.input === mode,
    )
  }, 'analysis source ' + mode)
  await analysisSource('main')
  await call(cameraPath + '/analysis-source', 'PUT', { mode: 'substream', source: rtsp })
  await analysisSource('substream')
  await call(cameraPath + '/analysis-source', 'PUT', { mode: 'substream', source: rtsp + '-missing' })
  await analysisSource('main')
  await call(cameraPath + '/analysis-source', 'PUT', { mode: 'relay' })
  await analysisSource('relay')
  console.log('PASS · analysis substream, direct fallback and shared relay')
  browser = await chromium.launch({ args: ['--autoplay-policy=no-user-gesture-required'] })
  const admin = await browser.newPage()
  admin.on('console', message => { if (message.type() === 'error') logs.push(message.text()) })
  await admin.addInitScript(secret => {
    localStorage.setItem('admin_token', secret)
    localStorage.setItem('ls-welcome-done', '1')
  }, adminToken)
  await admin.goto(origin + '/#/wall')
  const frameReady = () => {
    const video = document.querySelector('.camera-preview video')
    return !!video && video.videoWidth > 0 && video.currentTime > 0
  }
  await admin.waitForFunction(frameReady, null, { timeout: 25000 })
  console.log('PASS · WebRTC decoded frame')

  await call('/api/go2rtc/config', 'PUT', { enabled: false })
  await analysisSource('main')
  await admin.locator('.camera-preview img').waitFor({ timeout: 15000 })
  assert.ok((await admin.locator('.snapshot-time').textContent()).match(/\d/))
  await call('/api/go2rtc/config', 'PUT', { enabled: true })
  await admin.waitForFunction(frameReady, null, { timeout: 25000 })
  console.log('PASS · authenticated snapshot fallback and gateway recovery')

  const share = await call('/api/share-links', 'POST', { name: 'Playback test', ttl_hours: 24 })
  const viewer = await browser.newPage()
  viewer.on('console', message => { if (message.type() === 'error') logs.push(message.text()) })
  const signaling = []
  viewer.on('websocket', socket => {
    if (!new URL(socket.url()).pathname.startsWith('/ws/webrtc/')) return
    for (const event of ['framesent', 'framereceived']) {
      socket.on(event, ({ payload }) => {
        if (signaling.length >= 24) return
        if (typeof payload !== 'string') {
          signaling.push([event, 'binary', payload.length])
          return
        }
        try {
          const { type, value } = JSON.parse(payload)
          signaling.push([event, type, type === 'mse' || type === 'error' ? value : undefined])
        } catch { signaling.push([event, 'other text']) }
      })
    }
    socket.on('close', () => signaling.push(['close']))
  })
  await viewer.goto(origin + share.url)
  try {
    await viewer.waitForFunction(frameReady, null, { timeout: 22000 })
  } catch (error) {
    const media = await viewer.evaluate(() => {
      const video = document.querySelector('.camera-preview video')
      return video ? {
        paused: video.paused, currentTime: video.currentTime, readyState: video.readyState,
        width: video.videoWidth, error: video.error?.message,
        buffered: Array.from({ length: video.buffered.length }, (_, i) => [video.buffered.start(i), video.buffered.end(i)]),
      } : { page: document.body.innerText.slice(0, 180) }
    })
    console.error('Share playback diagnostics:', JSON.stringify({ signaling, media }))
    throw error
  }
  await call('/api/share-links/' + share.id, 'DELETE')
  await viewer.waitForFunction(() => !document.querySelector('.camera-preview video')?.getAttribute('src'), null, { timeout: 6000 })
  const expiredSnapshot = await fetch(origin + cameraPath + '/snapshot', {
    headers: { 'X-Admin-Token': share.token }, signal: AbortSignal.timeout(5000),
  })
  assert.equal(expiredSnapshot.status, 401)
  await admin.waitForFunction(frameReady, null, { timeout: 6000 })
  console.log('PASS · MSE video decoded and disconnects upon share revocation')

  const localPassword = 'isolated-playback-test-password'
  await admin.goto(origin + '/#/settings/access')
  const settings = admin.locator('.access-form')
  await settings.getByLabel('访问密码', { exact: true }).fill(localPassword)
  await settings.getByLabel('确认新密码', { exact: true }).fill(localPassword)
  await settings.getByRole('button', { name: '保存访问设置' }).click()
  await admin.getByText('访问设置已生效。').waitFor({ timeout: 15000 })

  const protectedViewer = await browser.newPage()
  await protectedViewer.goto(origin + '/#/wall')
  await protectedViewer.getByRole('heading', { name: '登录' }).waitFor()
  assert.equal(await protectedViewer.locator('.camera-preview').count(), 0)
  await protectedViewer.getByLabel('访问密码', { exact: true }).fill(localPassword)
  await protectedViewer.getByRole('button', { name: '登录', exact: true }).click()
  await protectedViewer.waitForFunction(frameReady, null, { timeout: 25000 })
  assert.match(await protectedViewer.locator('.camera-preview video').getAttribute('src'), /^blob:/)
  const session = (await protectedViewer.context().cookies()).find(item => item.name === 'ls_session')
  assert.ok(session?.httpOnly && session.sameSite === 'Strict')

  await admin.reload()
  await admin.getByRole('button', { name: '撤销其他会话' }).click()
  await protectedViewer.waitForFunction(() => !document.querySelector('.camera-preview video')?.getAttribute('src'), null, { timeout: 6000 })
  const afterRevocation = await protectedViewer.request.get(origin + '/api/system')
  assert.equal(afterRevocation.status(), 401)

  const hiddenCamera = await call('/api/cameras', 'POST', { name: 'Separate scope', type: 'file', source: 'unused.mp4' })
  await admin.goto(origin + '/#/settings/devices')
  const pairingForm = admin.locator('.device-form')
  await pairingForm.getByLabel('摄像头范围').selectOption({ label: '指定摄像头与分组' })
  await pairingForm.locator(`input[type=checkbox][value="${camera.id}"]`).check()
  await pairingForm.getByRole('button', { name: '生成配对码' }).click()
  await pairingForm.locator('img[alt="设备配对二维码"]').waitFor({ timeout: 15000 })
  const pairingLink = await pairingForm.getByLabel('配对链接').inputValue()
  const device = await browser.newPage({ viewport: { width: 375, height: 840 } })
  await device.goto(pairingLink)
  await device.getByLabel('设备名称').fill('Isolated QA tablet')
  await device.getByRole('button', { name: '配对并进入值守' }).click()
  await device.waitForFunction(frameReady, null, { timeout: 22000 })
  assert.equal(await device.locator('.camera-preview').count(), 1)
  assert.equal((await device.request.get(`${origin}/api/cameras/${hiddenCamera.id}/snapshot`)).status(), 403)
  const paired = await call('/api/devices')
  const tablet = paired.find(item => item.name === 'Isolated QA tablet')
  assert.ok(tablet?.id)
  await call('/api/devices/' + tablet.id, 'DELETE')
  await device.waitForFunction(() => !document.querySelector('.camera-preview video')?.getAttribute('src'), null, { timeout: 6000 })
  await device.getByRole('heading', { name: '设备配对已失效' }).waitFor({ timeout: 10000 })
  assert.equal((await device.request.get(`${origin}/api/cameras`)).status(), 401)
  console.log('PASS · browser pairing, scoped live camera and revocation')

  await admin.getByRole('button', { name: '退出登录' }).click()
  await admin.getByRole('heading', { name: '登录' }).waitFor()
  console.log('PASS · localhost password, browser session, revocable media and sign-out')
} catch (err) {
  console.error(logs.join('\n'))
  throw err
} finally {
  await browser?.close()
  if (server && server.exitCode === null) {
    try {
      const exited = once(server, 'exit')
      await call('/api/system/shutdown', 'POST', {})
      await Promise.race([exited, delay(5000)])
    } catch { /* terminate only the test child */ }
  }
  await stop(server)
  await stop(producer)
  rmSync(work, { recursive: true, force: true, maxRetries: 8, retryDelay: 200 })
}
