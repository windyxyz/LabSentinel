#!/usr/bin/env node
/**
 * go2rtc 信令 mock —— 最小实现，离线验证 WebrtcPlayer 组件（不依赖真实 go2rtc）
 * ==============================================================================
 *
 * 用途：
 *   在 /api/ws 上按 go2rtc 官方 demo 的 WS 信令协议回应——收到 {type:'webrtc'}
 *   的 offer 后，回一条【格式合法的假 answer SDP】（base64 编码文本，回显 offer
 *   的 m 行/mid，附 ice-ufrag/pwd/fingerprint），再附送一个假服务器候选。
 *   ICE 不会真通，但足以验证组件的：信令握手、onmessage 解析、
 *   addIceCandidate 容错、以及断流后的指数退避重连路径。
 *
 * 启动（在项目根目录）：
 *   node web/mock/go2rtc-mock.mjs                 # 默认监听 1984 端口（go2rtc 默认端口）
 *   node web/mock/go2rtc-mock.mjs --port 1984 --close-after 300
 *       # --close-after N：回完 answer 后 N 毫秒主动断开，模拟"流中断"
 *
 * 内置自测（无需浏览器）：
 *   node web/mock/go2rtc-mock.mjs --selftest
 *       临时端口上起服务端 + 起一个 1:1 复刻 WebrtcPlayer.vue 信令序列与
 *       退避参数的客户端：发 offer → 收假 answer（校验格式）→ 主动断开 →
 *       观察退避间隔递增（0.5s 起步 ×2、上限 15s）→ 中途模拟 ontrack 成功
 *       验证退避计数重置。
 *
 * 依赖：ws（MIT 协议；已装入 web devDependencies，
 *       安装：cd web && npm i -D ws --registry=https://registry.npmmirror.com）
 *
 * 组件用法见 web/src/components/WebrtcPlayer.vue 顶部注释。
 */

import http from 'node:http'
import crypto from 'node:crypto'
import { WebSocketServer, WebSocket } from 'ws'

/* ================= 参数解析 ================= */

const argv = process.argv.slice(2)
const has = (name) => argv.includes(name)
const argVal = (name, def) => {
  const i = argv.indexOf(name)
  return i >= 0 && argv[i + 1] !== undefined ? Number(argv[i + 1]) || argv[i + 1] : def
}

const PORT = Number(argVal('--port', 1984))
const CLOSE_AFTER = Number(argVal('--close-after', 0)) || 0

const ts = () => {
  const d = new Date()
  return `${d.toTimeString().slice(0, 8)}.${String(d.getMilliseconds()).padStart(3, '0')}`
}
const log = (tag, ...rest) => console.log(`[${ts()}] [${tag}]`, ...rest)

/* ================= 假 SDP 生成 ================= */

const randomHex = (bytes) => crypto.randomBytes(bytes).toString('hex')
// 伪 DTLS 指纹：sha-256 后跟 32 个冒号分隔的大写十六进制字节
const fakeFingerprint = () =>
  Array.from(crypto.randomBytes(32), (b) => b.toString(16).padStart(2, '0').toUpperCase()).join(':')

/**
 * 依据 offer 生成格式合法的假 answer：
 * 回显 offer 的每个 m= 行（保持编解码 payload 与 mid 对应），方向改为 sendonly，
 * 会话级补 BUNDLE 组行。浏览器 setRemoteDescription 能通过解析（ICE 不会真通）。
 */
function makeAnswerSdp(offerSdp) {
  const lines = offerSdp.split(/\r?\n/).filter(Boolean)
  const mLines = lines.filter((l) => l.startsWith('m='))
  const mids = lines.filter((l) => l.startsWith('a=mid:')).map((l) => l.slice('a=mid:'.length))
  const s = ['v=0', `o=go2rtc-mock ${Math.floor(Date.now() / 1000)} 1 IN IP4 127.0.0.1`, 's=go2rtc-mock', 't=0 0']
  if (lines.some((l) => l.startsWith('a=group:BUNDLE')) && mids.length) {
    s.push(`a=group:BUNDLE ${mids.join(' ')}`)
  }
  mLines.forEach((m, i) => {
    s.push(m) // 原样回显媒体行（payload 与 offer 一致，浏览器才能匹配编解码）
    s.push('c=IN IP4 0.0.0.0')
    s.push('a=rtcp:9 IN IP4 0.0.0.0')
    s.push(`a=ice-ufrag:${randomHex(2)}`)
    s.push(`a=ice-pwd:${randomHex(11)}`)
    s.push(`a=fingerprint:sha-256 ${fakeFingerprint()}`)
    s.push('a=setup:passive')
    s.push(`a=mid:${mids[i] ?? String(i)}`)
    s.push('a=sendonly')
    s.push('a=rtcp-mux')
  })
  return s.join('\r\n') + '\r\n'
}

// 自测客户端用的"浏览器形状"假 offer（video + audio 两路 recvonly）
function makeFakeOfferSdp() {
  const b64 = (n) => crypto.randomBytes(n).toString('base64url')
  return [
    'v=0',
    `o=- ${Date.now()} 2 IN IP4 127.0.0.1`,
    's=-',
    't=0 0',
    'a=group:BUNDLE 0 1',
    'a=msid-semantic: WMS',
    'm=video 9 UDP/TLS/RTP/SAVPF 96',
    'c=IN IP4 0.0.0.0',
    'a=rtcp:9 IN IP4 0.0.0.0',
    `a=ice-ufrag:${b64(2)}`,
    `a=ice-pwd:${b64(11)}`,
    `a=fingerprint:sha-256 ${fakeFingerprint()}`,
    'a=setup:actpass',
    'a=mid:0',
    'a=recvonly',
    'a=rtcp-mux',
    'a=rtpmap:96 H264/90000',
    'm=audio 9 UDP/TLS/RTP/SAVPF 111',
    'c=IN IP4 0.0.0.0',
    'a=rtcp:9 IN IP4 0.0.0.0',
    `a=ice-ufrag:${b64(2)}`,
    `a=ice-pwd:${b64(11)}`,
    `a=fingerprint:sha-256 ${fakeFingerprint()}`,
    'a=setup:actpass',
    'a=mid:1',
    'a=recvonly',
    'a=rtcp-mux',
    'a=rtpmap:111 opus/48000/2',
  ].join('\r\n') + '\r\n'
}

/* ================= 服务端 ================= */

/** 启动 mock 服务端，返回 { port, close }；port=0 时由系统分配临时端口 */
function startServer({ port = PORT, closeAfter = CLOSE_AFTER } = {}) {
  return new Promise((resolve, reject) => {
    let connSeq = 0
    let answered = 0

    const httpServer = http.createServer((req, res) => {
      // 简单健康检查页（go2rtc 本体在 / 有 UI，这里只给文字说明）
      res.writeHead(200, { 'Content-Type': 'text/plain; charset=utf-8' })
      res.end('go2rtc-mock：信令 WS 地址 ws://<host>:' + port + '/api/ws?src=<stream>\n')
    })

    const wss = new WebSocketServer({ noServer: true })
    httpServer.on('upgrade', (req, socket, head) => {
      const u = new URL(req.url, 'http://localhost')
      if (u.pathname !== '/api/ws') { socket.destroy(); return }
      wss.handleUpgrade(req, socket, head, (wsC) => wss.emit('connection', wsC, req))
    })

    wss.on('connection', (wsC, req) => {
      const id = ++connSeq
      const u = new URL(req.url, 'http://localhost')
      log('mock', `连接 #${id} 建立 src=${u.searchParams.get('src') ?? '(无)'} 来自 ${req.socket.remoteAddress}`)

      wsC.on('message', (data) => {
        let msg
        try { msg = JSON.parse(data.toString()) } catch { return log('mock', `#${id} 非法 JSON，忽略`) }

        if (msg.type === 'webrtc' && msg.value) {
          const offer = Buffer.from(msg.value, 'base64').toString('utf8')
          const mCount = offer.split(/\r?\n/).filter((l) => l.startsWith('m=')).length
          const firstSdpLine = offer.split(/\r?\n/).find(Boolean)
          log('mock', `#${id} 收到 offer（${offer.length} 字节，${mCount} 个 m 行，首行 "${firstSdpLine}"）→ 回假 answer`)
          const answer = makeAnswerSdp(offer)
          wsC.send(JSON.stringify({ type: 'webrtc', value: Buffer.from(answer, 'utf8').toString('base64') }))
          answered += 1
          // 附送一个假服务器候选，让客户端走 addIceCandidate 路径（地址不可达，ICE 不会真通）
          const cand = 'candidate:1 1 udp 2130706431 192.168.1.50 40000 typ host generation 0'
          wsC.send(JSON.stringify({ type: 'webrtc/candidate', value: Buffer.from(cand, 'utf8').toString('base64') }))
          log('mock', `#${id} 已发送 answer + 1 个假候选${closeAfter ? `，${closeAfter}ms 后主动断开` : ''}`)
          if (closeAfter > 0) setTimeout(() => { try { wsC.close() } catch { /* 忽略 */ } }, closeAfter)
        } else if (msg.type === 'webrtc/candidate') {
          const cand = msg.value ? Buffer.from(msg.value, 'base64').toString('utf8') : '(空 = 收集完成)'
          log('mock', `#${id} 收到客户端候选: ${cand}`)
        } else {
          log('mock', `#${id} 未处理消息 type=${msg.type}`)
        }
      })
      wsC.on('close', () => log('mock', `#${id} 连接关闭`))
      wsC.on('error', (e) => log('mock', `#${id} 连接错误: ${e.message}`))
    })

    httpServer.on('error', reject)
    httpServer.listen(port, () => resolve({
      port: httpServer.address().port,
      close: () => new Promise((r) => httpServer.close(r)),
      get answered() { return answered },
    }))
  })
}

/* ================= 自测客户端（1:1 复刻 WebrtcPlayer.vue 的信令与退避） ================= */

// 与 WebrtcPlayer.vue 保持一致的退避参数
const BACKOFF_BASE = 500   // 首次重连延迟 ms
const BACKOFF_MAX = 15000  // 退避上限 ms
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const b64 = (s) => Buffer.from(s, 'utf8').toString('base64')
const unb64 = (s) => Buffer.from(s, 'base64').toString('utf8')

/** 校验假 answer 的 SDP 格式，打印逐项结果，返回是否全部通过 */
function validateAnswerSdp(sdp) {
  const lines = sdp.split(/\r?\n/).filter(Boolean)
  const checks = {
    '首行 v=0': lines[0] === 'v=0',
    '含 o=/s=/t= 会话行': ['o=', 's=', 't=0 0'].every((p) => lines.some((l) => l.startsWith(p))),
    'm 行数量 = 2（video+audio）': lines.filter((l) => l.startsWith('m=')).length === 2,
    'mid 0/1 回显': lines.includes('a=mid:0') && lines.includes('a=mid:1'),
    '两个 m 行方向均为 sendonly': lines.filter((l) => l === 'a=sendonly').length === 2,
    '含 ice-ufrag 与 ice-pwd': lines.some((l) => l.startsWith('a=ice-ufrag:')) && lines.some((l) => l.startsWith('a=ice-pwd:')),
    '含 sha-256 指纹': lines.some((l) => l.startsWith('a=fingerprint:sha-256 ')),
    '含 a=setup:passive': lines.includes('a=setup:passive'),
    '含 a=rtcp-mux': lines.filter((l) => l === 'a=rtcp-mux').length === 2,
  }
  let allPass = true
  for (const [name, ok] of Object.entries(checks)) {
    log('自测', `answer 格式检查「${name}」：${ok ? '通过' : '失败'}`)
    allPass &&= ok
  }
  return allPass
}

/**
 * 一轮"组件式"连接：连 ws → 发 offer → 收 answer（校验）→ 收候选 → 主动断开。
 * 返回 { gotAnswer, answerOk, gotCandidate, closeT }。
 * round === RESET_ROUND 时在收到 answer 后模拟组件的 ontrack 成功（外部回调重置退避计数）。
 */
function connectOnce(url, round, resetHook) {
  return new Promise((resolve) => {
    const t0 = Date.now()
    let gotAnswer = false
    let answerOk = false
    let gotCandidate = false
    let done = false
    const finish = (reason) => {
      if (done) return
      done = true
      try { wsC.close() } catch { /* 忽略 */ }
      resolve({ gotAnswer, answerOk, gotCandidate, closeT: Date.now(), reason, liveMs: Date.now() - t0 })
    }

    const wsC = new WebSocket(url)
    wsC.on('open', () => {
      // 复刻组件 ws.onopen：createOffer → setLocalDescription → 发 offer
      const offer = makeFakeOfferSdp()
      wsC.send(JSON.stringify({ type: 'webrtc', value: b64(offer) }))
      log('自测', `→ 已发送 offer（${offer.length} 字节）`)
    })
    wsC.on('message', (data) => {
      const msg = JSON.parse(data.toString()) // 复刻组件 onmessage 的 JSON.parse
      if (msg.type === 'webrtc' && msg.value) {
        gotAnswer = true
        const sdp = unb64(msg.value) // 复刻组件 atob(msg.value)
        log('自测', `← 收到假 answer（${sdp.length} 字节）`)
        answerOk = validateAnswerSdp(sdp)
        if (round === RESET_ROUND && resetHook) {
          resetHook() // 模拟组件 ontrack：attempts 归零
          log('自测', `>>> 第 ${RESET_ROUND} 轮模拟 ontrack 成功：退避计数已重置为 0`)
        }
        // 复刻"流中断后组件主动拆链"：收到 answer 后短暂存活即主动断开
        setTimeout(() => finish('主动断开（模拟流中断）'), 150)
      } else if (msg.type === 'webrtc/candidate' && msg.value) {
        gotCandidate = true
        log('自测', `← 收到服务端候选: ${unb64(msg.value).slice(0, 48)}...`)
      }
    })
    wsC.on('error', (e) => log('自测', `ws 错误: ${e.message}`))
    wsC.on('close', () => finish('对端关闭'))
  })
}

const RESET_ROUND = 4 // 第几轮模拟 ontrack 成功（重置退避计数）
const ROUNDS = 7      // 总轮数：3 次递增 → 重置 → 2 次递增 → 末轮连上即止

async function selftest() {
  console.log('========== go2rtc-mock 自测：信令握手 + 指数退避重连 ==========')
  const server = await startServer({ port: 0, closeAfter: 0 })
  const url = `ws://127.0.0.1:${server.port}/api/ws?src=1`
  log('自测', `mock 服务端已起在临时端口 ${server.port}`)

  // 组件同款状态：attempts 为连续失败计数，ontrack 成功归零
  let attempts = 0
  let lastClose = null
  const records = [] // { round, expected, measured }
  let answerOkAll = true
  let gotCandidateAll = true

  for (let round = 1; round <= ROUNDS; round++) {
    const label = attempts === 0 ? '首次连接' : `第 ${attempts} 次重连`
    const r = await connectOnce(url, round, () => { attempts = 0 })
    answerOkAll &&= r.answerOk
    gotCandidateAll &&= r.gotCandidate
    log('自测', `第 ${round} 轮连接结束（存活 ${r.liveMs}ms，${r.reason}）`)
    lastClose = r.closeT
    attempts += 1 // 复刻组件 ws.onclose → scheduleReconnect 的 attempts += 1
    if (round < ROUNDS) {
      const delay = Math.min(BACKOFF_BASE * 2 ** (attempts - 1), BACKOFF_MAX)
      log('自测', `连接断开 → ${delay}ms 后发起${attempts === 0 ? '重置后的首次' : `第 ${attempts} 次`}重连`)
      await sleep(delay)
      records.push({ round: round + 1, expected: delay, measured: Date.now() - lastClose })
    }
  }

  /* ---------- 汇总 ---------- */
  console.log('\n---------- 自测结论 ----------')
  log('自测', `信令握手（offer 发出 → 假 answer 收到，共 ${ROUNDS} 轮）：${server.answered >= ROUNDS ? '通过' : '失败'}`)
  log('自测', `answer SDP 格式逐项校验：${answerOkAll ? '通过' : '失败'}`)
  log('自测', `服务端假候选接收（addIceCandidate 路径）：${gotCandidateAll ? '通过' : '失败'}`)
  log('自测', '断开→下次连接的退避间隔（实测 vs 理论）：')
  console.log('  轮次   实测间隔   理论退避')
  for (const rec of records) {
    console.log(`  ${String(rec.round).padStart(3)}    ${String(rec.measured).padStart(5)}ms   ${String(rec.expected).padStart(5)}ms${rec.round === RESET_ROUND + 1 ? `   ← 第 ${RESET_ROUND} 轮 ontrack 重置生效，回到起步值` : ''}`)
  }
  const theory = Array.from({ length: 8 }, (_, i) => Math.min(BACKOFF_BASE * 2 ** i, BACKOFF_MAX))
  log('自测', `理论退避序列（连续失败 1..8 次，ms）：${theory.join(', ')}（第 6 次起被 ${BACKOFF_MAX}ms 上限封顶）`)
  log('自测', `服务端共应答 offer ${server.answered} 次`)

  await server.close()
  const pass = answerOkAll && gotCandidateAll && server.answered >= ROUNDS
  console.log(`========== 自测${pass ? '通过' : '失败'}，退出码 ${pass ? 0 : 1} ==========`)
  process.exit(pass ? 0 : 1)
}

/* ================= 入口 ================= */

if (has('--selftest')) {
  selftest().catch((e) => { console.error(e); process.exit(1) })
} else {
  const server = await startServer()
  log('mock', `go2rtc-mock 已启动：ws://localhost:${server.port}/api/ws?src=<stream>  （Ctrl+C 停止）`)
  log('mock', `将地址填入 WebrtcPlayer 的 ws-url 即可离线联调${CLOSE_AFTER ? `；当前 --close-after=${CLOSE_AFTER}ms，会周期性断流以触发组件重连` : ''}`)

  const shutdown = async () => {
    log('mock', `收到退出信号，本次运行共应答 offer ${server.answered} 次，关闭服务端`)
    await server.close()
    process.exit(0)
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)
}
