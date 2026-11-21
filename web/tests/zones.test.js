// 2026-11-21 回归：安全区页闪黑/空白根因是 /api/calendar 与 /api/rule-templates
// 返回数组，而前端按 {exceptions}/{templates} 解包 → undefined.length 渲染崩溃。
// 本测试锁定「数组契约 + 包裹形状容错」下页面必须正常渲染。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import Zones from '../src/views/Zones.vue'

function mockFetch(responses) {
  const fn = vi.fn().mockImplementation((url, opts = {}) => {
    const key = `${opts.method || 'GET'} ${url.split('?')[0]}`
    const body = responses[key]
    if (body === undefined) {
      return Promise.resolve({ ok: false, status: 404, json: async () => ({}) })
    }
    return Promise.resolve({
      ok: true, status: 200, json: async () => (typeof body === 'function' ? body() : body),
    })
  })
  vi.stubGlobal('fetch', fn)
  return fn
}

afterEach(() => { vi.unstubAllGlobals() })

const BASE = {
  'GET /api/cameras': [{ id: 1, name: '测试相机' }],
  'GET /api/zones': [{ id: 7, camera_id: 1, name: '试剂区', polygon: [[1, 2], [3, 4], [5, 6]] }],
  'GET /api/rules': [],
  'GET /api/calendar': [],
  'GET /api/rule-templates': [],
}

describe('Zones.vue 渲染健壮性', () => {
  it('数组契约：页面完整渲染（不再 undefined.length 崩溃）', async () => {
    mockFetch(BASE)
    const w = mount(Zones)
    await flushPromises()
    expect(w.text()).toContain('已配置安全区')
    expect(w.text()).toContain('试剂区')
    expect(w.text()).toContain('复制到')
  })

  it('兼容 {exceptions}/{templates} 包裹形状', async () => {
    mockFetch({
      ...BASE,
      'GET /api/calendar': { exceptions: [{ date: '2026-12-25', mode: 'holiday', note: '' }] },
      'GET /api/rule-templates': {
        templates: [{ id: 1, name: '标准模板', config: { rules: [{}, {}] } }],
      },
    })
    const w = mount(Zones)
    await flushPromises()
    expect(w.text()).toContain('2026-12-25')
    expect(w.text()).toContain('标准模板')
    expect(w.text()).toContain('2 条')
  })

  it('异常数据（polygon 非数组 / zones 非数组）不炸页面', async () => {
    mockFetch({
      ...BASE,
      'GET /api/zones': [{ id: 9, camera_id: 1, name: '坏数据', polygon: 'oops' }],
      'GET /api/rules': {},
      'GET /api/calendar': {},
      'GET /api/rule-templates': {},
    })
    const w = mount(Zones)
    await flushPromises()
    expect(w.text()).toContain('已配置安全区')
    expect(w.text()).toContain('坏数据')
    expect(w.text()).toContain('顶点数')
  })
})
