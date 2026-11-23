// R2-502：Wall 值班大屏（网格上限 9、事件滚动）+ Models 模型下载确认流程。
// WebrtcPlayer 在 jsdom 无 RTCPeerConnection/WebSocket——测试先挂最小 stub。
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { api, adminToken } from '../src/api'

function mockFetch(responses) {
  const fn = vi.fn().mockImplementation((url, opts = {}) => {
    const key = `${opts.method || 'GET'} ${url.split('?')[0]}`
    const body = responses[key]
    if (body === undefined) return Promise.resolve({ ok: false, status: 404, json: async () => ({}) })
    return Promise.resolve({
      ok: true, status: 200, json: async () => (typeof body === 'function' ? body() : body),
    })
  })
  vi.stubGlobal('fetch', fn)
  return fn
}

// jsdom 缺失的浏览器 API 最小 stub（WebrtcPlayer 构造/信令不会真正跑通，能挂载即可）
function stubWebApis() {
  vi.stubGlobal('RTCPeerConnection', class {
    constructor() { this.localDescription = { sdp: '' } }
    addTransceiver() {}
    createOffer() { return Promise.resolve({ sdp: '' }) }
    setLocalDescription() { return Promise.resolve() }
    setRemoteDescription() { return Promise.resolve() }
    addIceCandidate() { return Promise.resolve() }
    close() {}
  })
  vi.stubGlobal('WebSocket', class {
    constructor() { this.readyState = 1 }
    send() {}
    close() {}
    addEventListener() {}
  })
  vi.stubGlobal('atob', (s) => Buffer.from(s, 'base64').toString('binary'))
  vi.stubGlobal('btoa', (s) => Buffer.from(s, 'binary').toString('base64'))
}

beforeEach(() => { stubWebApis(); adminToken.set('adm') })
afterEach(() => {
  vi.unstubAllGlobals()
  adminToken.set('')
})

describe('Wall.vue（T-709）', () => {
  it('超过 9 路时只渲染前 9 格（网格上限）', async () => {
    const cams = Array.from({ length: 12 }, (_, i) => ({
      id: i + 1, name: `cam${i + 1}`, type: 'folder', status: 'stopped',
    }))
    mockFetch({
      'GET /api/cameras': cams,
      'GET /api/events': [],
    })
    const Wall = (await import('../src/views/Wall.vue')).default
    const w = mount(Wall)
    await flushPromises()
    const cells = w.findAll('.grid .cell')
    expect(cells.length).toBe(9)
    expect(w.text()).toContain('cam9')
    expect(w.text()).not.toContain('cam10')
  })

  it('事件滚动最多保留 50 条', async () => {
    mockFetch({
      'GET /api/cameras': [],
      'GET /api/events': Array.from({ length: 60 }, (_, i) => ({
        id: i + 1, ts: '2026-11-09T10:00:00', type: 'enter', level: 2, detail: `d${i}`,
      })),
    })
    const Wall = (await import('../src/views/Wall.vue')).default
    const w = mount(Wall)
    await flushPromises()
    expect(w.findAll('aside ul li').length).toBe(50)
  })
})

describe('Models.vue（模型安装）', () => {
  it('缺少随包模型时可上传自定义模型或用安装包修复', async () => {
    mockFetch({
      'GET /api/models/ppe/status': {
        installed: false, mode: 'person_only', license_confirmed_required: true,
        license_notice: 'Roboflow 数据集许可待确认', sha256: 'ab'.repeat(32), size_bytes: 10_673_073,
      },
    })
    const Models = (await import('../src/views/Models.vue')).default
    const w = mount(Models)
    await flushPromises()
    expect(w.text()).toContain('仅人员检测')
    expect(w.text()).toContain('使用完整安装包修复检测')
    expect(w.text()).toContain('上传替换模型')
    expect(w.findAll('button').some((b) => b.text().includes('下载'))).toBe(false)
  })
})
