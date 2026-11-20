// UX1/UX2（2026-11-20）：检测模式前端契约——DetectPanel 三模式卡片切换 /
// API 服务卡 / 协作策略保存 / 预算展示 + api.js detect* URL 抽检。
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { api, adminToken } from '../src/api'
import DetectPanel from '../src/views/DetectPanel.vue'

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

const CONFIG = {
  mode: 'local',
  vlm: { provider: 'openai_compat', url: '', model: 'gpt-4o-mini',
         api_key_set: false, api_key: '***', timeout_s: 30, allow_remote: false },
  cascade: { trigger: 'both', conf_lo: 0.35, conf_hi: 0.65, daily_budget: 200, on_negative: 'keep' },
  usage: { date: '2026-11-20', count: 0, budget: 200, exhausted: false },
}

beforeEach(() => { adminToken.set('adm') })
afterEach(() => {
  vi.unstubAllGlobals()
  adminToken.set('')
})

describe('api.js detect* 契约', () => {
  it('URL 与方法抽检', async () => {
    const fn = mockFetch({ 'GET /api/detect/config': CONFIG, 'GET /api/detect/usage': {} })
    await api.detectConfig()
    await api.detectUsage()
    const calls = fn.mock.calls.map((c) => `${c[1]?.method || 'GET'} ${c[0].split('?')[0]}`)
    expect(calls).toContain('GET /api/detect/config')
    expect(calls).toContain('GET /api/detect/usage')

    const fn2 = mockFetch({ 'PUT /api/detect/config': { ok: true } })
    await api.saveDetectConfig({ mode: 'cascade' })
    const fn3 = mockFetch({ 'POST /api/detect/test': { ok: true } })
    await api.detectTest()
    const urls = [...fn2.mock.calls, ...fn3.mock.calls].map((c) => c[0].split('?')[0])
    expect(urls).toContain('/api/detect/config')
    expect(urls).toContain('/api/detect/test')
  })
})

describe('DetectPanel.vue（UX2）', () => {
  it('三模式卡片渲染且本地模式默认选中', async () => {
    mockFetch({ 'GET /api/detect/config': CONFIG })
    const w = mount(DetectPanel)
    await flushPromises()
    const cards = w.findAll('[role="radio"]')
    expect(cards.length).toBe(3)
    expect(cards[0].attributes('aria-checked')).toBe('true')  // local 默认
    expect(cards[2].text()).toContain('本地 + API 协作')
    expect(w.text()).toContain('今日 API 复核')
  })

  it('点协作卡片即 PUT mode=cascade（即时生效契约）', async () => {
    const fn = mockFetch({
      'GET /api/detect/config': {
        ...CONFIG,
        vlm: { ...CONFIG.vlm, url: 'http://127.0.0.1:8899' }, // 回环已配置 → 允许切换
      },
      'PUT /api/detect/config': { ok: true, restarted: 2 },
    })
    const w = mount(DetectPanel)
    await flushPromises()
    await w.findAll('[role="radio"]')[2].trigger('click')
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT')
    expect(JSON.parse(put[1].body).mode).toBe('cascade')
    expect(w.text()).toContain('2 路跟随全局的相机已重启生效')
  })

  it('远程 URL 未勾选合规确认时拒绝切换 API 模式（前端预检）', async () => {
    const fn = mockFetch({
      'GET /api/detect/config': {
        ...CONFIG,
        vlm: { ...CONFIG.vlm, url: 'https://api.example.com/v1', allow_remote: false },
      },
    })
    const w = mount(DetectPanel)
    await flushPromises()
    await w.findAll('[role="radio"]')[1].trigger('click') // 仅 API
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT')
    expect(put).toBeUndefined()
    expect(w.text()).toContain('合规') // 引导文案
  })

  it('API 配置保存：api_key 留空不上送（不变更语义）', async () => {
    const fn = mockFetch({
      'GET /api/detect/config': {
        ...CONFIG, mode: 'api',
        vlm: { ...CONFIG.vlm, url: 'http://127.0.0.1:8899', api_key_set: true },
      },
      'PUT /api/detect/config': { ok: true },
    })
    const w = mount(DetectPanel)
    await flushPromises()
    const saveBtn = w.findAll('button').find((b) => b.text() === '保存 API 配置')
    await saveBtn.trigger('click')
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT')
    const body = JSON.parse(put[1].body)
    expect(body.vlm.url).toBe('http://127.0.0.1:8899')
    expect(body.vlm.api_key).toBeUndefined() // 未输入新值=不变更
  })

  it('预算耗尽展示降级横幅', async () => {
    mockFetch({
      'GET /api/detect/config': {
        ...CONFIG,
        usage: { date: '2026-11-20', count: 200, budget: 200, exhausted: true },
      },
    })
    const w = mount(DetectPanel)
    await flushPromises()
    expect(w.text()).toContain('预算已用尽')
    expect(w.text()).toContain('仅本地')
  })
})
