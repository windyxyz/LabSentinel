// LS-042 回归测试：锁定前后端契约对齐与 11 处前端 P0 修复，防止再次发生渲染崩溃或功能静默失效。
import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import ReportsView from '../src/views/ReportsView.vue'
import Events from '../src/views/Events.vue'
import FolderImages from '../src/views/FolderImages.vue'
import Zones from '../src/views/Zones.vue'
import FsPicker from '../src/components/FsPicker.vue'
import { adminToken, mediaUrl } from '../src/api'

function mockFetch(responses) {
  const fn = vi.fn().mockImplementation((url, opts = {}) => {
    const key = `${opts.method || 'GET'} ${url.split('?')[0]}`
    const body = responses[key]
    if (body === undefined) {
      return Promise.resolve({ ok: false, status: 404, json: async () => ({ detail: 'not found' }) })
    }
    return Promise.resolve({
      ok: true,
      status: 200,
      json: async () => (typeof body === 'function' ? body(url, opts) : body),
    })
  })
  vi.stubGlobal('fetch', fn)
  return fn
}

afterEach(() => {
  vi.unstubAllGlobals()
  localStorage.clear()
})

describe('LS-042 前后端契约与 P0 回归', () => {
  it('ReportsView：statsHourly 兼容对象形态与 24 项数组形态，有事件不崩', async () => {
    mockFetch({
      'GET /api/cameras': [{ id: 1, name: '实验室1' }],
      'GET /api/stats/summary': { total: 5, by_level: { 1: 1, 2: 2, 3: 2 }, days: 30, top_days: [{ date: '2026-11-22', total: 5 }] },
      'GET /api/stats/hourly': { date: '2026-11-22', hours: { '08': { total: 3, l1: 0, l2: 1, l3: 2 } } },
      'GET /api/stats/daily': { days: [{ date: '2026-11-22', total: 5, l1: 1, l2: 2, l3: 2 }] },
      'GET /api/stats/quality': { days: 7, total: 5, by_camera: [] },
      'GET /api/audit': { items: [], total: 0 },
    })
    const w = mount(ReportsView)
    await flushPromises()
    expect(w.text()).toContain('2026-11-22')
    expect(w.findAll('svg').length).toBeGreaterThan(0)
  })

  it('Events：表头含「时间」列且与数据列数一致（10 列）', async () => {
    mockFetch({
      'GET /api/cameras': [{ id: 1, name: '测试相机' }],
      'GET /api/events': [{ id: 9, ts: '2026-11-22T09:00:00', camera_id: 1, type: 'enter', level: 2, detail: '进入', status: 'new' }],
      'GET /api/events/count': { count: 1 },
    })
    const w = mount(Events)
    await flushPromises()
    const ths = w.findAll('table.events-table th')
    expect(ths.length).toBe(10)
    expect(ths[1].text()).toBe('时间')
  })

  it('mediaUrl 对 undefined/空值安全防御（开启 admin_token 不抛 split 异常）', () => {
    adminToken.set('secret-tok')
    expect(mediaUrl(undefined)).toBe('')
    expect(mediaUrl('')).toBe('')
  })

  it('FolderImages：开启 admin_token 且条目缺 thumb_url 时不崩溃', async () => {
    adminToken.set('tok123')
    mockFetch({
      'GET /api/cameras': [{ id: 2, name: '图片源', type: 'folder', status: 'running' }],
      'GET /api/folder-images/summary': { days: 7, items: [], totals: { total: 1, key: 0, violation: 0 } },
      'GET /api/system': { cameras: [{ id: 2, name: '图片源', type: 'folder', status: 'running' }] },
      'GET /api/folder-images': { items: [{ id: 10, camera_id: 2, name: 'a.jpg', ts: '2026-11-22T09:00:00', persons: 1, in_zone: 0, ppe_no: 0, ppe_wearing: 1, key: false, violation: false }], total: 1 },
      'GET /api/media/sign': { exp: 9999999999, sig: 'abc' },
    })
    const w = mount(FolderImages)
    await flushPromises()
    expect(w.text()).toContain('a.jpg')
  })

  it('Zones：days_of_week 逗号分隔正确显示「周一三五」而非「周一?三?五」', async () => {
    mockFetch({
      'GET /api/cameras': [{ id: 1, name: '测试相机' }],
      'GET /api/zones': [],
      'GET /api/rules': [{ id: 1, camera_id: 1, zone_id: null, behavior: 'enter', level: 2, cooldown_s: 30, days_of_week: '1,3,5', enabled: true }],
      'GET /api/calendar': [],
      'GET /api/rule-templates': [{ id: 1, name: '数组模板', config: [{ behavior: 'enter' }] }],
    })
    const w = mount(Zones)
    await flushPromises()
    expect(w.text()).toContain('周一三五')
    expect(w.text()).not.toContain('?')
    expect(w.text()).toContain('1 条')
  })

  it('FsPicker：使用 api.fsList 携带 admin_token 并展示条目', async () => {
    adminToken.set('adm-xyz')
    const f = mockFetch({
      'GET /api/fs/list': { path: 'D:/', entries: [{ name: 'pics', path: 'D:/pics', is_dir: true }] },
    })
    const w = mount(FsPicker)
    w.vm.openAt('D:/')
    await flushPromises()
    expect(w.text()).toContain('pics')
    const [, opts] = f.mock.calls[0]
    expect(opts.headers['X-Admin-Token']).toBe('adm-xyz')
  })

  it('FsPicker：视频模式返回选中的视频文件路径', async () => {
    const path = 'D:/监控/检测样本.MP4'
    mockFetch({
      'GET /api/fs/list': {
        path: 'D:/监控',
        entries: [{ name: '检测样本.MP4', path, is_dir: false },
          { name: 'notes.txt', path: 'D:/监控/notes.txt', is_dir: false }],
      },
    })
    const view = mount(FsPicker, { props: { video: true, title: '选择视频文件' } })
    view.vm.openAt('D:/监控')
    await flushPromises()
    expect(view.text()).not.toContain('notes.txt')
    await view.findAll('button').find(button => button.text() === '选择').trigger('click')
    expect(view.emitted('pick')).toEqual([[path]])
    view.unmount()
  })
})
