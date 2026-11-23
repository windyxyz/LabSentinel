// 0.4.0：实验服模型管理——启停开关 / 上传替换 / 恢复随包。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import Models from '../src/views/Models.vue'

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

const STATUS = {
  installed: true, enabled: true, source: 'bundled', bundled: true,
  mode: 'ppe', active: true, license_confirmed_required: false,
  license_notice: '随安装包提供', sha256: 'f00231e6', size_bytes: 10673073,
  model_path: 'yolo11n_ppe.onnx', note: '已启用',
  // LS-050：阈值与类别清单
  violation_conf: 0.3, wearing_conf: 0.2,
  classes: ['person', 'lab coat', 'no lab coat', 'no_gloves'],
}

afterEach(() => { vi.unstubAllGlobals() })

describe('Models.vue（0.4.0）', () => {
  it('单独启用随包的近景口罩分类', async () => {
    const fn = mockFetch({
      'GET /api/models/ppe/status': { ...STATUS, close_mask_available: true, close_mask_enabled: false },
      'PUT /api/models/ppe/config': { ok: true, close_mask_enabled: true, restarted: 0 },
    })
    const w = mount(Models)
    await flushPromises()
    const checkbox = w.get('input[aria-label="启用近景口罩检测"]')
    expect(checkbox.element.checked).toBe(false)
    await checkbox.setValue(true)
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT')
    expect(JSON.parse(put[1].body)).toEqual({ close_mask_enabled: true })
  })

  it('关闭 PPE 检测：PUT /api/models/ppe/config enabled=false', async () => {
    const fn = mockFetch({
      'GET /api/models/ppe/status': STATUS,
      'PUT /api/models/ppe/config': { ok: true, enabled: false, restarted: 1 },
    })
    const w = mount(Models)
    await flushPromises()
    expect(w.text()).toContain('随安装包提供')
    const cb = w.find('input[type="checkbox"]')
    cb.element.checked = false
    await cb.trigger('change')
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT')
    expect(put[0]).toContain('/api/models/ppe/config')
    expect(JSON.parse(put[1].body).enabled).toBe(false)
    expect(w.text()).toContain('已关闭 PPE 检测')
  })

  it('上传替换：POST multipart 到 /api/models/ppe/upload', async () => {
    vi.stubGlobal('confirm', () => true)
    const fn = mockFetch({
      'GET /api/models/ppe/status': { ...STATUS, source: 'custom' },
      'POST /api/models/ppe/upload': { ok: true, restarted: 0 },
    })
    const w = mount(Models)
    await flushPromises()
    const input = w.find('input[type="file"]')
    const file = new File([new Uint8Array([1, 2, 3])], 'ppe.onnx', { type: 'application/octet-stream' })
    Object.defineProperty(input.element, 'files', { value: [file] })
    await input.trigger('change')
    await flushPromises()
    const post = fn.mock.calls.find((c) => c[1]?.method === 'POST')
    expect(post[0]).toContain('/api/models/ppe/upload')
    expect(post[1].body).toBeInstanceOf(FormData)
    expect(w.text()).toContain('已上传并启用')
  })

  it('恢复随包模型：POST /api/models/ppe/reset', async () => {
    vi.stubGlobal('confirm', () => true)
    const fn = mockFetch({
      'GET /api/models/ppe/status': { ...STATUS, source: 'custom' },
      'POST /api/models/ppe/reset': { ok: true, restarted: 0 },
    })
    const w = mount(Models)
    await flushPromises()
    const btn = w.findAll('button').find((b) => b.text() === '恢复随包模型')
    expect(btn.attributes('disabled')).toBeUndefined()
    await btn.trigger('click')
    await flushPromises()
    const post = fn.mock.calls.find((c) => c[1]?.method === 'POST')
    expect(post[0]).toContain('/api/models/ppe/reset')
    expect(w.text()).toContain('已恢复随包模型')
  })

  it('LS-050：展示 PPE 阈值滑块与类别清单', async () => {
    mockFetch({ 'GET /api/models/ppe/status': STATUS })
    const w = mount(Models)
    await flushPromises()
    expect(w.text()).toContain('违规置信度')
    expect(w.text()).toContain('穿戴置信度')
    expect(w.text()).toContain('no_gloves')
    expect(w.findAll('input[type="range"]').length).toBe(2)
  })

  it('LS-050：拖动阈值滑块保存 violation_conf', async () => {
    const fn = mockFetch({
      'GET /api/models/ppe/status': STATUS,
      'PUT /api/models/ppe/config': { ok: true, restarted: 0 },
    })
    const w = mount(Models)
    await flushPromises()
    const sliders = w.findAll('input[type="range"]')
    sliders[0].element.value = '0.45'
    await sliders[0].trigger('change')
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT')
    expect(JSON.parse(put[1].body).violation_conf).toBe(0.45)
  })
})
