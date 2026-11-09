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

describe('Models.vue（Q-08 下载确认流程）', () => {
  it('未勾选许可确认时不发下载请求', async () => {
    const fetch = mockFetch({
      'GET /api/models/ppe/status': {
        installed: false, mode: 'person_only', license_confirmed_required: true,
        license_notice: 'Roboflow 数据集许可待确认', sha256: 'ab'.repeat(32), size_bytes: 10_673_073,
      },
    })
    const Models = (await import('../src/views/Models.vue')).default
    const w = mount(Models)
    await flushPromises()
    expect(w.text()).toContain('仅人员检测')
    // 找下载按钮并点击（未勾选 confirm）
    const btn = w.findAll('button').find((b) => /下载|启用/.test(b.text()))
    if (btn) {
      await btn.trigger('click')
      await flushPromises()
    }
    const dl = fetch.mock.calls.find(([u, o]) => (o?.method || 'GET') === 'POST')
    expect(dl).toBeUndefined()
  })

  it('勾选许可后发起下载并展示结果', async () => {
    const fetch = mockFetch({
      'GET /api/models/ppe/status': {
        installed: false, mode: 'person_only', license_confirmed_required: true,
        license_notice: 'Roboflow 数据集许可待确认', sha256: 'ab'.repeat(32), size_bytes: 10_673_073,
      },
      'POST /api/models/ppe/download': { installed: true, mode: 'ppe' },
    })
    const Models = (await import('../src/views/Models.vue')).default
    const w = mount(Models)
    await flushPromises()
    // 勾选确认复选框
    const cb = w.find('input[type="checkbox"]')
    expect(cb.exists()).toBe(true)
    await cb.setValue(true)
    const btn = w.findAll('button').find((b) => /下载|启用/.test(b.text()))
    expect(btn).toBeTruthy()
    await btn.trigger('click')
    await flushPromises()
    const dl = fetch.mock.calls.find(([u, o]) => (o?.method || 'GET') === 'POST'
      && u.includes('/api/models/ppe/download'))
    expect(dl).toBeTruthy()
    expect(JSON.parse(dl[1].body).confirm_license).toBe(true)
    expect(w.text()).toContain('人员 + 实验服检测')
  })
})
