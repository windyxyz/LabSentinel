// R2-502 余量收口：WebrtcPlayer 子协议鉴权 / 信令 offer-answer / 断线退避重连 /
// ICE 观察窗 / 卸载清理——RTCPeerConnection 与 WebSocket 用可控 stub 全流程覆盖。
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import WebrtcPlayer from '../src/components/WebrtcPlayer.vue'
import { adminToken } from '../src/api'

/** 可控的 WebSocket stub：记录构造参数与 send 内容，手动驱动 onopen/onmessage/onclose。 */
class FakeWebSocket {
  static OPEN = 1
  static created = [] // { url, protocols, instance }
  constructor(url, protocols) {
    this.url = url
    this.protocols = protocols
    this.readyState = 0 // CONNECTING，由测试 open() 置 OPEN
    this.sent = []
    this.closed = false
    FakeWebSocket.created.push({ url, protocols, instance: this })
  }
  send(data) { this.sent.push(JSON.parse(data)) }
  close() { this.closed = true }
  /** 测试驱动：连接打开 */
  open() { this.readyState = FakeWebSocket.OPEN; if (this.onopen) this.onopen() }
  /** 测试驱动：收到服务端消息 */
  message(obj) { if (this.onmessage) this.onmessage({ data: JSON.stringify(obj) }) }
  /** 测试驱动：连接关闭 */
  fail() { this.readyState = 3; if (this.onclose) this.onclose() }
}

/** 可控的 RTCPeerConnection stub：暴露事件 handler 供测试触发。 */
class FakePC {
  static created = []
  constructor() {
    this.localDescription = { type: 'offer', sdp: 'OFFER-SDP' }
    this.iceConnectionState = 'new'
    this.connectionState = 'new'
    this.setLocalCalls = []
    this.setRemoteCalls = []
    this.addIceCalls = []
    this.closed = false
    FakePC.created.push(this)
  }
  addTransceiver() {}
  createOffer() { return Promise.resolve({ type: 'offer', sdp: 'OFFER-SDP' }) }
  setLocalDescription(desc) { this.setLocalCalls.push(desc); return Promise.resolve() }
  setRemoteDescription(desc) { this.setRemoteCalls.push(desc); return Promise.resolve() }
  addIceCandidate(c) { this.addIceCalls.push(c); return Promise.resolve() }
  close() { this.closed = true }
}

async function mountPlayer(wsUrl = '/ws/webrtc/1') {
  const w = mount(WebrtcPlayer, { props: { wsUrl, label: 'cam1' } })
  await flushPromises()
  return w
}

function latestWs() { return FakeWebSocket.created[FakeWebSocket.created.length - 1] }

beforeEach(() => {
  FakeWebSocket.created = []
  FakePC.created = []
  vi.stubGlobal('WebSocket', FakeWebSocket)
  vi.stubGlobal('RTCPeerConnection', FakePC)
  vi.stubGlobal('btoa', (s) => Buffer.from(s, 'binary').toString('base64'))
  vi.stubGlobal('atob', (s) => Buffer.from(s, 'base64').toString('binary'))
  // jsdom 的 HTMLMediaElement.play 未实现（返回 undefined），ontrack 里 .catch 会炸——桩成 Promise
  window.HTMLMediaElement.prototype.play = () => Promise.resolve()
  adminToken.set('adm')
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
  adminToken.set('')
})

describe('WebrtcPlayer.vue（R2-502 收口）', () => {
  it('T-102/R2-104：相对路径按同源组装 ws://；有令牌时以 labsentinel + bearer.<t> 双子协议连接', async () => {
    await mountPlayer()
    expect(FakeWebSocket.created).toHaveLength(1)
    const { url, protocols } = FakeWebSocket.created[0]
    expect(url).toBe('ws://localhost:3000/ws/webrtc/1')
    expect(protocols).toEqual(['labsentinel', 'bearer.adm'])
  })

  it('无令牌时不携带子协议（RFC 6455：请求了不回显的子协议会被浏览器断开，故不请求）', async () => {
    adminToken.set('')
    await mountPlayer()
    expect(FakeWebSocket.created[0].protocols).toBeUndefined()
  })

  it('信令 offer：onopen 后 createOffer→setLocalDescription→发送 {type:webrtc,value:base64(sdp)}', async () => {
    await mountPlayer()
    latestWs().instance.open()
    await flushPromises()
    const pc = FakePC.created[0]
    expect(pc.setLocalCalls).toHaveLength(1)
    expect(latestWs().instance.sent).toEqual([
      { type: 'webrtc', value: Buffer.from('OFFER-SDP', 'binary').toString('base64') },
    ])
  })

  it('answer 处理：base64 sdp → setRemoteDescription({type:answer})；ontrack → 已连接 + attempts 清零', async () => {
    vi.useFakeTimers()
    const w = await mountPlayer()
    latestWs().instance.open()
    await flushPromises()
    latestWs().instance.message({ type: 'webrtc', value: Buffer.from('ANSWER-SDP', 'binary').toString('base64') })
    await flushPromises()
    const pc = FakePC.created[0]
    expect(pc.setRemoteCalls).toEqual([{ type: 'answer', sdp: 'ANSWER-SDP' }])
    expect(w.text()).not.toContain('已连接')
    // ontrack：真正拿到流才算连接成功（DOM 更新异步，先 flush 再断言）
    const stream = { id: 's1' }
    pc.ontrack({ streams: [stream], track: {} })
    await flushPromises()
    expect(w.text()).toContain('已连接')
    // 连接成功后断开 → 退避计数已清零，首次重连仍是 500ms 基数
    latestWs().instance.fail()
    await flushPromises()
    expect(w.text()).toContain('重连中(第1次)')
    vi.advanceTimersByTime(500)
    expect(FakeWebSocket.created).toHaveLength(2)
  })

  it('断线指数退避：500ms → 1000ms（BACKOFF_BASE×2^n）', async () => {
    vi.useFakeTimers()
    await mountPlayer()
    latestWs().instance.fail()          // 第 1 次断开
    expect(latestWs().instance.closed).toBe(true)
    vi.advanceTimersByTime(499)
    expect(FakeWebSocket.created).toHaveLength(1)
    vi.advanceTimersByTime(1)           // 500ms 到点
    expect(FakeWebSocket.created).toHaveLength(2)
    FakeWebSocket.created[1].instance.fail()
    vi.advanceTimersByTime(1000)        // 第 2 次：1000ms
    expect(FakeWebSocket.created).toHaveLength(3)
  })

  it('OFFER_TIMEOUT_MS：发出 offer 后 10s 未收到 answer → 退避重连', async () => {
    vi.useFakeTimers()
    await mountPlayer()
    latestWs().instance.open()
    await flushPromises()
    vi.advanceTimersByTime(9999)
    expect(FakeWebSocket.created).toHaveLength(1)
    vi.advanceTimersByTime(1)      // 超时到点 → scheduleReconnect（先退避 500ms 再 connect）
    expect(FakeWebSocket.created).toHaveLength(1)
    vi.advanceTimersByTime(500)
    expect(FakeWebSocket.created).toHaveLength(2)
    expect(FakeWebSocket.created[0].instance.closed).toBe(true)
  })

  it('ICE 观察窗：failed 持续 3s 才重连；窗口内 connected 恢复则不重连', async () => {
    vi.useFakeTimers()
    await mountPlayer()
    latestWs().instance.open()
    await flushPromises()
    const pc = FakePC.created[0]
    // failed → 安排观察窗，窗口未满不重连
    pc.iceConnectionState = 'failed'
    pc.oniceconnectionstatechange()
    vi.advanceTimersByTime(2999)
    expect(FakeWebSocket.created).toHaveLength(1)
    vi.advanceTimersByTime(1)      // 观察窗到点 → scheduleReconnect（再 500ms 后 connect）
    vi.advanceTimersByTime(500)
    expect(FakeWebSocket.created).toHaveLength(2)
    // 新会话：窗口期内恢复 connected → 清除观察窗，不重连
    FakeWebSocket.created[1].instance.open()
    await flushPromises()
    const pc2 = FakePC.created[1]
    pc2.iceConnectionState = 'disconnected'
    pc2.oniceconnectionstatechange()
    pc2.iceConnectionState = 'connected'
    pc2.oniceconnectionstatechange()
    vi.advanceTimersByTime(5000)
    expect(FakeWebSocket.created).toHaveLength(2)
  })

  it('卸载清理：关 ws 与 pc，销毁后不再重连', async () => {
    vi.useFakeTimers()
    const w = await mountPlayer()
    const first = latestWs().instance
    w.unmount()
    expect(first.closed).toBe(true)
    expect(FakePC.created[0].closed).toBe(true)
    vi.advanceTimersByTime(30000)
    expect(FakeWebSocket.created).toHaveLength(1)
  })
})
