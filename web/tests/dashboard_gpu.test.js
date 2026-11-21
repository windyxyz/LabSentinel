// R2-303 → UX（2026-11-21）：Dashboard GPU 状态卡三态（配置偏好 providers +
// gpu_active 实际生效——DML 失败自动回落 CPU 时如实显示）。
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import Dashboard from '../src/views/Dashboard.vue'
import { adminToken } from '../src/api'

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

function systemPayload(providers, gpu_active = null) {
  return {
    version: '0.2.0', backend: providers.includes('DmlExecutionProvider') ? 'directml' : 'cpu',
    providers, gpu_pref: providers.includes('DmlExecutionProvider') ? 'dml' : 'cpu', gpu_active,
    today_counts: { '1': 2 }, unhandled_today: 1,
    cameras: [], alerts: { pending: 0, failed: 0, ok: 0 },
    config_issues: {}, components: {},
  }
}

beforeEach(() => adminToken.set('adm'))
afterEach(() => {
  vi.unstubAllGlobals()
  adminToken.set('')
})

describe('Dashboard.vue GPU 状态（UX 三态）', () => {
  it('CPU 偏好（无 Dml）→ 显示仅 CPU + 切换引导', async () => {
    mockFetch({
      'GET /api/system': systemPayload(['AzureExecutionProvider', 'CPUExecutionProvider']),
      'GET /api/maintenance/status': {},
    })
    const w = mount(Dashboard)
    await flushPromises()
    expect(w.text()).toContain('cpu')
    expect(w.text()).toContain('推理设备：CPU')
    expect(w.text()).toContain('设置→模型与检测')
  })

  it('DML 已配置但未启动相机（无 gpu_active）→ 已配置待生效', async () => {
    mockFetch({
      'GET /api/system': systemPayload(['DmlExecutionProvider', 'CPUExecutionProvider']),
      'GET /api/maintenance/status': {},
    })
    const w = mount(Dashboard)
    await flushPromises()
    expect(w.text()).toContain('directml')
    expect(w.text()).toContain('GPU 加速：已配置（DirectML）')
    expect(w.text()).toContain('启动相机后生效')
  })

  it('DML 实际生效（gpu_active=DirectML(GPU)）→ 已启用', async () => {
    mockFetch({
      'GET /api/system': systemPayload(['DmlExecutionProvider', 'CPUExecutionProvider'], 'DirectML(GPU)'),
      'GET /api/maintenance/status': {},
    })
    const w = mount(Dashboard)
    await flushPromises()
    expect(w.text()).toContain('GPU 加速：已启用（DirectML(GPU)）')
    expect(w.text()).not.toContain('回落')
  })

  it('DML 失败回落（gpu_active=CPU）→ 如实显示回落 + 切换引导', async () => {
    mockFetch({
      'GET /api/system': systemPayload(['DmlExecutionProvider', 'CPUExecutionProvider'], 'CPU'),
      'GET /api/maintenance/status': {},
    })
    const w = mount(Dashboard)
    await flushPromises()
    expect(w.text()).toContain('推理设备：CPU（GPU 初始化失败已自动回落')
    expect(w.text()).toContain('设置→模型与检测')
  })
})
