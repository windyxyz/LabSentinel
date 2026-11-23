import { afterEach, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import Face from '../src/views/Face.vue'

afterEach(() => vi.unstubAllGlobals())

it('switches on a local model, checks consent and tests the latest sample', async () => {
  let mode = 'off'
  const config = () => ({
    active: { mode, api: '' }, status: { mode, enabled: mode !== 'off', configured: mode === 'local' },
    local: { installed: true, enabled: true, threshold: 0.5 }, apis: [], cameras: [], retention_days: 90,
  })
  const fetch = vi.fn(async (path, options) => {
    const ok = value => ({ ok: true, json: async () => value })
    if (path === '/api/face/config') {
      if (options.method === 'PUT') mode = JSON.parse(options.body).mode
      return ok(config())
    }
    if (path === '/api/face/library') return ok([])
    if (path === '/api/face/recognize') {
      expect(options.body.get('consent')).toBe('true')
      expect(options.body.get('photo').name).toBe('test.jpg')
      return ok({ matched: { subject: '合成测试', similarity: 0.81 }, elapsed_ms: 18 })
    }
    throw new Error('Unexpected request')
  })
  const NativeURL = URL
  vi.stubGlobal('URL', class extends NativeURL {
    static createObjectURL() { return 'blob:test-face' }
    static revokeObjectURL() {}
  })
  vi.stubGlobal('fetch', fetch)
  const view = mount(Face)
  const button = label => view.findAll('button').find(candidate => candidate.text() === label)
  try {
    await flushPromises()
    expect(view.text()).toContain('启用配置完整的模型后可录入')
    await button('本地 YuNet + SFace').trigger('click')
    await flushPromises()
    const files = view.get('input[type=file]')
    Object.defineProperty(files.element, 'files', { value: [new File(['test'], 'test.jpg', { type: 'image/jpeg' })] })
    await files.trigger('change')
    expect(button('试识别最后一张').attributes('disabled')).toBeDefined()
    await view.get('.enroll-form input[type=checkbox]').setValue(true)
    await button('试识别最后一张').trigger('click')
    await flushPromises()
    expect(view.text()).toContain('合成测试 · 相似度 0.81')
    await button('关闭').trigger('click')
    await flushPromises()
    expect(view.find('input[type=file]').exists()).toBe(false)
  } finally { view.unmount() }
})
