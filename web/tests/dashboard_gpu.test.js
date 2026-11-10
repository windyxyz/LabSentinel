// R2-303：Dashboard GPU 加速状态卡（依据 /api/system.providers 探测 DmlExecutionProvider）。
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

function systemPayload(providers) {
  return {
    version: '0.2.0', backend: providers.includes('DmlExecutionProvider') ? 'directml' : 'cpu',
    providers, today_counts: { '1': 2 }, unhandled_today: 1,
    cameras: [], alerts: { pending: 0, failed: 0, ok: 0 },
    config_issues: {}, components: {},
  }
}

beforeEach(() => adminToken.set('adm'))
afterEach(() => {
  vi.unstubAllGlobals()
  adminToken.set('')
})

describe('Dashboard.vue GPU 加速提示（R2-303）', () => {
  it('CPU 变体（无 DmlExecutionProvider）→ 显示未启用 + GPU 变体提示', async () => {
    mockFetch({
      'GET /api/system': systemPayload(['AzureExecutionProvider', 'CPUExecutionProvider']),
      'GET /api/maintenance/status': {},
    })
    const w = mount(Dashboard)
    await flushPromises()
    expect(w.text()).toContain('cpu')
    expect(w.text()).toContain('GPU 加速：未启用')
    expect(w.text()).toContain('可选 GPU(DML) 变体包随 0.3.0 提供')
  })

  it('DML 变体 → 显示开启与实际 provider 名', async () => {
    mockFetch({
      'GET /api/system': systemPayload(['DmlExecutionProvider', 'CPUExecutionProvider']),
      'GET /api/maintenance/status': {},
    })
    const w = mount(Dashboard)
    await flushPromises()
    expect(w.text()).toContain('directml')
    expect(w.text()).toContain('GPU 加速：开启（DmlExecutionProvider）')
    expect(w.text()).not.toContain('未启用')
  })
})
