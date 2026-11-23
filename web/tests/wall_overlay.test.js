// LS-051：VideoOverlayCanvas 坐标缩放 / 合规态颜色 + Wall 宫格与静音开关。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import { imagePoint, mapBox, stateColor } from '../src/overlay.js'

describe('VideoOverlayCanvas 纯函数', () => {
  it('标注坐标在缩放平移后仍对应原图像素，并限制在原图边界内', () => {
    const zoomed = { left: -40, top: -80, width: 2400, height: 1200 }
    expect(imagePoint(1160, 520, zoomed, 800, 400)).toEqual({ x: 400, y: 200 })
    expect(imagePoint(9000, -100, zoomed, 800, 400)).toEqual({ x: 800, y: 0 })
  })
  it('mapBox：等比缩放并居中（letterbox）', () => {
    // 1280x720 源 → 640x360 容器：scale=0.5，无偏移
    const r = mapBox([100, 50, 300, 250], 1280, 720, 640, 360)
    expect(r.scale).toBeCloseTo(0.5, 5)
    expect(r.x).toBeCloseTo(50, 5)
    expect(r.y).toBeCloseTo(25, 5)
    expect(r.w).toBeCloseTo(100, 5)
    expect(r.h).toBeCloseTo(100, 5)
  })

  it('mapBox：容器更宽时左右留边', () => {
    // 1280x720 → 1280x720 容器高度受限：scale=0.5, offX=(1280-640)/2=320
    const r = mapBox([0, 0, 1280, 720], 1280, 720, 1280, 360)
    expect(r.scale).toBeCloseTo(0.5, 5)
    expect(r.offX).toBeCloseTo(320, 5)
    expect(r.x).toBeCloseTo(320, 5)
  })

  it('mapBox：非法输入返回 null', () => {
    expect(mapBox(null, 1280, 720, 640, 360)).toBeNull()
    expect(mapBox([1, 2, 3], 1280, 720, 640, 360)).toBeNull()
    expect(mapBox([1, 2, 3, 4], 0, 720, 640, 360)).toBeNull()
    expect(mapBox([NaN, 2, 3, 4], 1280, 720, 640, 360)).toBeNull()
  })

  it('stateColor：绿/红/灰三态', () => {
    expect(stateColor('compliant')).toBe('#22c55e')
    expect(stateColor('violation')).toBe('#ef4444')
    expect(stateColor('pending')).toBe('#9ca3af')
    expect(stateColor(undefined)).toBe('#9ca3af')
  })
})

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

afterEach(() => { vi.unstubAllGlobals() })

describe('Wall.vue（LS-051 全屏值守）', () => {
  it('默认 9 宫格 + 检测框显隐 + 静音开关', async () => {
    vi.stubGlobal('WebSocket', class {
      constructor() { this.readyState = 0 }
      close() {}
      set onmessage(_) {}
    })
    vi.stubGlobal('requestAnimationFrame', () => 0)
    vi.stubGlobal('cancelAnimationFrame', () => {})
    mockFetch({
      'GET /api/cameras': [],
      'GET /api/events': [],
      'GET /api/stats/summary': { today_total: 10, today_compliant: 9 },
    })
    const Wall = (await import('../src/views/Wall.vue')).default
    const w = mount(Wall)
    await flushPromises()
    expect(w.text()).toContain('进入全屏值守大屏')
    expect(w.text()).toContain('隐藏检测框')
    expect(w.text()).toContain('开启提示音')
    // 默认 9 宫格
    expect(w.find('select[aria-label="宫格切换"]').element.value).toBe('9')
    // 切到 4 宫格
    await w.find('select[aria-label="宫格切换"]').setValue('4')
    expect(w.find('select[aria-label="宫格切换"]').element.value).toBe('4')
    // 静音 → 开启提示音
    const muteBtn = w.findAll('button').find((b) => b.text() === '开启提示音')
    await muteBtn.trigger('click')
    expect(w.text()).toContain('静音')
  })
})

it('分享链接校验后锁定值守视图并保留本机管理令牌', async () => {
  history.replaceState(null, '', '/#/wall?readonly_token=share-test')
  localStorage.setItem('admin_token', 'local-admin')
  const fetch = mockFetch({ 'GET /api/share-links/verify': { valid: true } })
  const App = (await import('../src/App.vue')).default
  const wallStub = { props: ['readonly'], template: '<div>{{ readonly ? "只读视图" : "值守视图" }}</div>' }
  const view = mount(App, { global: { stubs: { Wall: wallStub } } })
  try {
    await flushPromises()
    expect(view.text()).toBe('只读视图')
    expect(view.find('nav').exists()).toBe(false)
    expect(fetch.mock.calls[0][1].headers['X-Admin-Token']).toBe('share-test')
    expect(localStorage.getItem('admin_token')).toBe('local-admin')
    expect(location.hash).toBe('#/wall')
  } finally {
    view.unmount()
    history.replaceState(null, '', '/')
    sessionStorage.removeItem('ls_readonly_token')
    localStorage.removeItem('admin_token')
  }
})

it('启用本机密码后在登录前隐藏产品视图，登录后用会话访问', async () => {
  const App = (await import('../src/App.vue')).default
  localStorage.setItem('admin_token', 'previous-browser-token')
  let signedIn = false
  const fetch = vi.fn(async (url, opts) => {
    if (url === '/api/access/login') {
      expect(JSON.parse(opts.body).password).toBe('local-fixture-password')
      expect(opts.headers['X-Admin-Token']).toBeUndefined()
      signedIn = true
      return { ok: true, json: async () => ({ ok: true }) }
    }
    expect(url).toBe('/api/access/status')
    return { ok: true, json: async () => ({ required: true, signed_in: signedIn, local: true, password_supported: true }) }
  })
  vi.stubGlobal('fetch', fetch)
  const view = mount(App, { global: { stubs: { Insights: true, Welcome: true } } })
  try {
    expect(view.find('nav').exists()).toBe(false)
    await flushPromises()
    expect(view.get('h1').text()).toBe('登录')
    expect(view.find('nav').exists()).toBe(false)
    await view.get('input[type="password"]').setValue('local-fixture-password')
    await view.get('form').trigger('submit')
    await flushPromises()
    expect(view.find('nav').exists()).toBe(true)
    expect(view.get('.sign-out').text()).toBe('退出登录')
  } finally {
    view.unmount()
    localStorage.removeItem('admin_token')
  }
})
