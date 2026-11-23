// R2-502：Devices 设备管理页契约（mock fetch，零网络）。
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { enableAutoUnmount, mount, flushPromises } from '@vue/test-utils'
import Devices from '../src/views/Devices.vue'
import { adminToken } from '../src/api'

vi.mock('qrcode', () => ({ default: { toDataURL: vi.fn().mockResolvedValue('data:image/png;base64,AA==') } }))
enableAutoUnmount(afterEach)

function mockFetch(responses) {
  // responses: { 'GET /api/devices': {...}, ... }
  const fn = vi.fn().mockImplementation((url, opts = {}) => {
    const key = `${opts.method || 'GET'} ${url}`
    const body = responses[key]
    if (!body) return Promise.resolve({ ok: false, status: 404, json: async () => ({}) })
    return Promise.resolve({
      ok: true, status: 200, json: async () => (typeof body === 'function' ? body() : body),
    })
  })
  vi.stubGlobal('fetch', fn)
  return fn
}

beforeEach(() => adminToken.set('adm'))
afterEach(() => {
  vi.unstubAllGlobals()
  adminToken.set('')
})

describe('Devices.vue', () => {
  it('空列表渲染提示 + 生成配对码显示 6 位码', async () => {
    const fetch = mockFetch({
      'GET /api/devices': [],
      'GET /api/cameras': [{ id: 12, type: 'rtsp', name: 'A room', group_name: 'Lab A' }],
      'POST /api/pairing': { code: '012345', expires_in_s: 300 },
    })
    const w = mount(Devices)
    await flushPromises()
    expect(w.text()).toContain('暂无配对设备')
    const form = w.get('.device-form')
    await form.findAll('select')[1].setValue('false')
    await form.get('input[value="Lab A"]').setValue(true)
    await form.trigger('submit')
    await flushPromises()
    expect(w.text()).toContain('012345')
    expect(w.text()).toContain('300 秒后到期')
    expect(w.get('.pair-url input').element.value).toContain('/#/pair?code=012345')
    const creation = fetch.mock.calls.find(([url]) => url === '/api/pairing')
    expect(JSON.parse(creation[1].body)).toEqual({
      role: 'read_only', all_cameras: false, camera_ids: [], groups: ['Lab A'], expires_days: 30,
    })
  })

  it('设备列表渲染 + 两步吊销', async () => {
    const fetch = mockFetch({
      'GET /api/devices': [
        { id: 1, name: '手机A', platform: 'android', last_seen_ts: null, revoked: false },
        { id: 2, name: '平板B', platform: 'ios', last_seen_ts: '2026-11-09T10:00:00', revoked: true },
      ],
      'DELETE /api/devices/1': { ok: true },
    })
    const w = mount(Devices)
    await flushPromises()
    expect(w.text()).toContain('手机A')
    expect(w.text()).toContain('平板B')
    expect(w.text()).toContain('已撤销')
    // 两步确认：先出现确认按钮，点击后才发 DELETE
    await w.findAll('button').find((b) => b.text() === '撤销设备').trigger('click')
    const delBtn = w.findAll('button').find((b) => b.text() === '确认撤销')
    expect(delBtn).toBeTruthy()
    await delBtn.trigger('click')
    await flushPromises()
    const del = fetch.mock.calls.find(([u, o]) => (o?.method || 'GET') === 'DELETE')
    expect(del).toBeTruthy()
    expect(del[0]).toBe('/api/devices/1')
  })
})
