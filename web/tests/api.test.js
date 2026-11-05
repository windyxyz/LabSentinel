// 任务133：api.js 契约测试（mock fetch，零网络）。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { api, adminToken, mediaUrl, connectEvents } from '../src/api'

function mockFetch(status, body) {
  const fn = vi.fn().mockResolvedValue({
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
  })
  vi.stubGlobal('fetch', fn)
  return fn
}

afterEach(() => {
  vi.unstubAllGlobals()
  adminToken.set('')
})

describe('adminToken / mediaUrl', () => {
  it('token 存取（空值清除）', () => {
    adminToken.set('abc')
    expect(adminToken.get()).toBe('abc')
    adminToken.set('')
    expect(adminToken.get()).toBe('')
  })

  it('mediaUrl：无 token 原样；有 token 按 ?/& 拼接', () => {
    expect(mediaUrl('/api/events/1/snapshot')).toBe('/api/events/1/snapshot')
    adminToken.set('t 1')
    expect(mediaUrl('/api/events/1/snapshot'))
      .toBe('/api/events/1/snapshot?token=t%201')
    expect(mediaUrl('/api/cameras/1/snapshot?x=1'))
      .toBe('/api/cameras/1/snapshot?x=1&token=t%201')
  })
})

describe('req 错误契约', () => {
  it('2xx 返回解析后的 JSON', async () => {
    mockFetch(200, { ok: 1 })
    await expect(api.system()).resolves.toEqual({ ok: 1 })
  })

  it('非 2xx 抛 detail', async () => {
    mockFetch(400, { detail: 'days must be 1-365' })
    await expect(api.statsQuality({})).rejects.toThrow('days must be 1-365')
  })

  it('401 追加令牌引导文案', async () => {
    mockFetch(401, { detail: 'nope' })
    await expect(api.pluginsList()).rejects.toThrow('需要管理令牌')
  })

  it('有令牌时带 X-Admin-Token 头；写请求带 JSON Content-Type', async () => {
    adminToken.set('sec')
    const fn = mockFetch(200, {})
    await api.addCamera({ name: 'x' })
    const [url, init] = fn.mock.calls[0]
    expect(url).toBe('/api/cameras')
    expect(init.headers['X-Admin-Token']).toBe('sec')
    expect(init.headers['Content-Type']).toBe('application/json')
    expect(init.body).toBe(JSON.stringify({ name: 'x' }))
  })
})

describe('URL 拼装（新增方法抽检）', () => {
  it.each([
    ['usbDevices', [], '/api/cameras/usb-devices'],
    ['usbDevices', [true], '/api/cameras/usb-devices?refresh=1'],
    ['cameraGroups', [], '/api/camera-groups'],
    ['pluginsList', [], '/api/plugins'],
  ])('%s', async (method, args, url) => {
    const fn = mockFetch(200, {})
    await api[method](...args)
    expect(fn.mock.calls[0][0]).toBe(url)
  })

  it.each([
    ['pluginAction', ['p1', 'enable'], '/api/plugins/p1/enable'],
    ['pluginAction', ['p2', 'stop'], '/api/plugins/p2/stop'],
    ['setCameraGroup', [3, 'g'], '/api/cameras/3/group'],
  ])('%s POST/PUT 语义', async (method, args, url) => {
    const fn = mockFetch(200, {})
    await api[method](...args)
    const [got, init] = fn.mock.calls[0]
    expect(got).toBe(url)
    expect(['POST', 'PUT']).toContain(init.method)
  })

  it('pluginLogs tail 参数', async () => {
    const fn = mockFetch(200, { logs: '' })
    await api.pluginLogs('p1', 50)
    expect(fn.mock.calls[0][0]).toBe('/api/plugins/p1/logs?tail=50')
  })
})

// T-106/T-201：WS 令牌走子协议（不进 URL），启用令牌后实时事件不失效
describe('connectEvents WS 子协议', () => {
  const makeWs = () => {
    const spy = vi.fn()
    class FakeWS {
      constructor(...args) { spy(...args); this.close = vi.fn(); this.onopen = null; this.onmessage = null; this.onclose = null }
    }
    vi.stubGlobal('WebSocket', FakeWS)
    return spy
  }

  it('无令牌时不传 protocols', () => {
    const wsSpy = makeWs()
    vi.stubGlobal('location', { protocol: 'http:', host: '127.0.0.1:8000' })
    const stop = connectEvents(() => {}, () => {})
    expect(wsSpy.mock.calls[0][0]).toBe('ws://127.0.0.1:8000/ws/events')
    expect(wsSpy.mock.calls[0][1]).toBeUndefined()
    stop()
  })

  it('有令牌时用 labsentinel + bearer.<token> 子协议', () => {
    const wsSpy = makeWs()
    vi.stubGlobal('location', { protocol: 'https:', host: 'ls.example' })
    adminToken.set('s3cret')
    const stop = connectEvents(() => {}, () => {})
    expect(wsSpy.mock.calls[0][0]).toBe('wss://ls.example/ws/events')
    expect(wsSpy.mock.calls[0][1]).toEqual(['labsentinel', 'bearer.s3cret'])
    stop()
  })
})
