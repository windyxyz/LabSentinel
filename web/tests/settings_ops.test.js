// LS-053：网络与运维（日报调度保存 / 局域网令牌 / 自启开关）。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import Settings from '../src/views/Settings.vue'

function mockFetch(responses) {
  const fn = vi.fn().mockImplementation((url, opts = {}) => {
    const key = `${opts.method || 'GET'} ${url.split('?')[0]}`
    const body = responses[key]
    if (body === undefined) return Promise.resolve({ ok: false, status: 404, json: async () => ({}) })
    return Promise.resolve({ ok: true, status: 200, json: async () => (typeof body === 'function' ? body() : body) })
  })
  vi.stubGlobal('fetch', fn)
  return fn
}

const BASE = {
  'GET /api/alerts/status': { channels: [] },
  'GET /api/plugins': { plugins: [] },
  'GET /api/cameras': [],
  'GET /api/alert-deliveries': [],
  'GET /api/alert-silences': [],
  'GET /api/alert-template': { title: '', body: '' },
  'GET /api/go2rtc/status': { enabled: false, running: false },
  'GET /api/system/network': { lan: false, host: '127.0.0.1', port: 8000, trusted_hosts: '', admin_token_set: false },
  'GET /api/system/autostart': { supported: true, enabled: false },
  'GET /api/reports/schedule': {
    daily_enabled: true, daily_time: '18:00', weekly_enabled: false,
    weekly_day: 1, weekly_time: '18:00', last_daily_date: '',
  },
}

afterEach(() => { vi.unstubAllGlobals() })

describe('Settings 网络与运维（LS-053）', () => {
  it('保存日报调度 PUT /api/reports/schedule', async () => {
    const fn = mockFetch({ ...BASE, 'PUT /api/reports/schedule': { ok: true } })
    const w = mount(Settings)
    await flushPromises()
    // 切到网络分区（sections 按钮按文字定位）
    const nav = w.findAll('.settings-item').find((b) => b.text().includes('网络与运维'))
    await nav.trigger('click')
    await flushPromises()
    const btn = w.findAll('button').find((b) => b.text() === '保存调度')
    await btn.trigger('click')
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT' && c[0].includes('/api/reports/schedule'))
    expect(put).toBeTruthy()
    expect(JSON.parse(put[1].body).daily_time).toBe('18:00')
  })

  it('生成强随机令牌并回显一次', async () => {
    const fn = mockFetch({
      ...BASE,
      'PUT /api/system/network': { ok: true, token: 'a'.repeat(64) },
    })
    const w = mount(Settings)
    await flushPromises()
    const nav = w.findAll('.settings-item').find((b) => b.text().includes('网络与运维'))
    await nav.trigger('click')
    await flushPromises()
    const btn = w.findAll('button').find((b) => b.text() === '生成强随机令牌')
    await btn.trigger('click')
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT' && c[0].includes('/api/system/network'))
    expect(JSON.parse(put[1].body).generate_token).toBe(true)
    expect(w.text()).toContain('a'.repeat(64))
  })

  it('局域网开关保存 lan=true', async () => {
    const fn = mockFetch({ ...BASE, 'PUT /api/system/network': { ok: true } })
    const w = mount(Settings)
    await flushPromises()
    const nav = w.findAll('.settings-item').find((b) => b.text().includes('网络与运维'))
    await nav.trigger('click')
    await flushPromises()
    const cb = w.findAll('input[type="checkbox"]').find((c) => c.element.closest('label')?.textContent.includes('允许局域网访问'))
    cb.element.checked = true
    await cb.trigger('change')
    const btn = w.findAll('button').find((b) => b.text() === '保存网络配置')
    await btn.trigger('click')
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT' && c[0].includes('/api/system/network'))
    expect(JSON.parse(put[1].body).lan).toBe(true)
  })
})
