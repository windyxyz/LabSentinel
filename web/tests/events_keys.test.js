// LS-052：事件流键盘快审（J/K/A/F/I/Space/Esc + 输入框聚焦不误触发）。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import Events from '../src/views/Events.vue'

function mockFetch(responses) {
  const fn = vi.fn().mockImplementation((url, opts = {}) => {
    const key = `${opts.method || 'GET'} ${url.split('?')[0]}`
    const body = responses[key]
    if (body === undefined) return Promise.resolve({ ok: false, status: 404, json: async () => ({}) })
    return Promise.resolve({ ok: true, status: 200, json: async () => body })
  })
  vi.stubGlobal('fetch', fn)
  return fn
}

const ROWS = [
  { id: 1, ts: '2026-11-22T10:00:00', camera_id: 1, type: 'enter', level: 2, detail: '事件A', status: 'new', has_snapshot: false, has_video: false },
  { id: 2, ts: '2026-11-22T10:01:00', camera_id: 1, type: 'exit', level: 2, detail: '事件B', status: 'new', has_snapshot: false, has_video: false },
  { id: 3, ts: '2026-11-22T10:02:00', camera_id: 1, type: 'enter', level: 2, detail: '事件C', status: 'new', has_snapshot: false, has_video: false },
]

afterEach(() => { vi.unstubAllGlobals() })

function key(k, extra = {}) {
  window.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true, ...extra }))
}

const baseResponses = () => ({
  'GET /api/events': ROWS,
  'GET /api/events/count': { count: 3 },
  'GET /api/cameras': [],
  'GET /api/events/timeline': { days: [] },
})

describe('Events.vue 键盘快审（LS-052）', () => {
  it('点击检查记录时，抽屉直接打开并显示图片预览', async () => {
    const response = baseResponses()
    response['GET /api/events'] = [{ ...ROWS[0], type: 'photo_check', category: 'check', status: 'recorded' }]
    mockFetch(response)
    const transition = vi.fn(update => {
      update()
      return { finished: Promise.resolve() }
    })
    document.startViewTransition = transition
    const w = mount(Events)
    try {
      await flushPromises()
      await w.get('.events-table tr:nth-child(2)').trigger('click')
      await flushPromises()
      expect(transition).not.toHaveBeenCalled()
      expect(document.querySelector('.drawer-snap')?.getAttribute('src')).toContain('/api/events/1/snapshot')
    } finally {
      delete document.startViewTransition
      w.unmount()
    }
  })

  it('J/K 上下切换抽屉，Esc 关闭', async () => {
    mockFetch(baseResponses())
    const w = mount(Events)
    await flushPromises()
    expect(document.querySelector('.drawer')).toBeNull()
    expect(w.text()).toContain('事件A') // 列表已加载
    key('j')
    await flushPromises()
    expect(document.querySelector('.drawer')).not.toBeNull()
    expect(document.querySelector('.drawer').textContent).toContain('事件A')
    key('j')
    await flushPromises()
    expect(document.querySelector('.drawer').textContent).toContain('事件B')
    key('k')
    await flushPromises()
    expect(w.text()).toContain('事件A')
    key('Escape')
    await flushPromises()
    expect(document.querySelector('.drawer')).toBeNull()
    w.unmount()
  })

  it('输入框聚焦时 J/K 不误触发', async () => {
    mockFetch(baseResponses())
    const w = mount(Events)
    await flushPromises()
    const input = document.createElement('input')
    document.body.appendChild(input)
    input.focus()
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'j', bubbles: true }))
    await flushPromises()
    expect(document.querySelector('.drawer')).toBeNull()
    input.remove()
    w.unmount()
  })

  it('页面展示快捷键提示条', async () => {
    mockFetch(baseResponses())
    const w = mount(Events)
    await flushPromises()
    expect(w.text()).toContain('下一条')
    expect(w.text()).toContain('误报')
    expect(w.text()).toContain('关闭')
    w.unmount()
  })
})
