// UX1/UX2（2026-11-20）：检测模式前端契约——DetectPanel 三模式卡片切换 /
// API 服务卡 / 协作策略保存 / 预算展示 + api.js detect* URL 抽检。
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { api, adminToken } from '../src/api'
import DetectPanel from '../src/views/DetectPanel.vue'

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

const CONFIG = {
  mode: 'local',
  vlm: { provider: 'openai_compat', url: '', model: 'gpt-4o-mini',
         api_key_set: false, api_key: '***', timeout_s: 30, allow_remote: false },
  cascade: { trigger: 'both', conf_lo: 0.35, conf_hi: 0.65, daily_budget: 200, on_negative: 'keep' },
  usage: { date: '2026-11-20', count: 0, budget: 200, exhausted: false },
}

beforeEach(() => { adminToken.set('adm') })
afterEach(() => {
  vi.unstubAllGlobals()
  adminToken.set('')
})

describe('api.js detect* 契约', () => {
  it('URL 与方法抽检', async () => {
    const fn = mockFetch({ 'GET /api/detect/config': CONFIG, 'GET /api/detect/usage': {} })
    await api.detectConfig()
    await api.detectUsage()
    const calls = fn.mock.calls.map((c) => `${c[1]?.method || 'GET'} ${c[0].split('?')[0]}`)
    expect(calls).toContain('GET /api/detect/config')
    expect(calls).toContain('GET /api/detect/usage')

    const fn2 = mockFetch({ 'PUT /api/detect/config': { ok: true } })
    await api.saveDetectConfig({ mode: 'cascade' })
    const fn3 = mockFetch({ 'POST /api/detect/test': { ok: true } })
    await api.detectTest()
    const urls = [...fn2.mock.calls, ...fn3.mock.calls].map((c) => c[0].split('?')[0])
    expect(urls).toContain('/api/detect/config')
    expect(urls).toContain('/api/detect/test')
  })
})

describe('DetectPanel.vue（UX2）', () => {
  it('按本机运行时枚举的编号提交预算并显示 CPU 回退原因', async () => {
    let status = {
      status: 'stopped',
      runtime_available: true,
      gpu_settings: { vulkan_device: 'auto', vram_budget_mb: 800, cv_vram_reserve_mb: 1024 },
      vulkan_devices: [null, { id: 'Vulkan2', name: 'Test GPU', free_mb: 768 }],
    }
    const fn = mockFetch({
      'GET /api/detect/config': CONFIG,
      'GET /api/detect/local-vlm': () => status,
      'POST /api/detect/local-vlm/start': () => {
        status = { ...status, status: 'cpu_fallback', fallback_reason: '显存不足' }
        return status
      },
    })
    const view = mount(DetectPanel)
    await flushPromises()
    const panel = view.findAll('.panel').find(candidate => candidate.text().includes('离线视觉服务'))
    const button = text => panel.findAll('button').find(candidate => candidate.text() === text)
    await panel.get('input[placeholder="视觉模型.gguf"]').setValue('model.gguf')
    await panel.get('input[placeholder="mmproj-F16.gguf"]').setValue('projector.gguf')
    await panel.get('input[max="128"]').setValue(12)
    await button('列出显卡').trigger('click')
    await flushPromises()
    await panel.get('select').setValue('Vulkan2')
    await button('启动').trigger('click')
    await flushPromises()
    expect(fn.mock.calls.some(([url]) => url.endsWith('/local-vlm?probe=true'))).toBe(true)
    const [, request] = fn.mock.calls.find(([url]) => url.endsWith('/local-vlm/start'))
    expect(JSON.parse(request.body)).toEqual({
      model: 'model.gguf', projector: 'projector.gguf', gpu_layers: 12,
      vulkan_device: 'Vulkan2', vram_budget_mb: 800, cv_vram_reserve_mb: 1024,
    })
    await vi.waitFor(() => expect(view.text()).toContain('已回退至 CPU'))
    expect(view.text()).toContain('显存不足')
    view.unmount()
  })

  it('仅灰度包显示 Vulkan 选项，按分析源显示人员与 PPE 的实际设备', async () => {
    let runtime = { ep: 'dml', backend: 'dml', vulkan_available: false }
    const fn = mockFetch({
      'GET /api/detect/config': () => ({ ...CONFIG, runtime }),
      'PUT /api/detect/config': { restarted: 1 },
    })
    const ordinary = mount(DetectPanel)
    await flushPromises()
    expect(ordinary.find('input[value="auto_vulkan"]').exists()).toBe(false)
    ordinary.unmount()

    runtime = { ...runtime, vulkan_available: true, gpu_id: 'auto' }
    const gray = mount(DetectPanel)
    await flushPromises()
    runtime = { ...runtime, backend: 'auto_vulkan', camera_devices: [
      { camera_id: 7, person: 'Vulkan(GPU)', ppe: 'DirectML(GPU)' },
    ] }
    await gray.get('input[value="auto_vulkan"]').setValue(true)
    await flushPromises()
    const put = fn.mock.calls.find(([, opts]) => opts?.method === 'PUT')
    expect(JSON.parse(put[1].body)).toEqual({ runtime: { backend: 'auto_vulkan' } })
    expect(gray.text()).toContain('#7 · 人员 Vulkan(GPU) · PPE DirectML(GPU)')
    gray.unmount()
  })

  it('选择物理显卡时保存 UUID 并显示同卡回退', async () => {
    const id = 'uuid:0102030405060708090a0b0c0d0e0f10'
    const runtime = {
      backend: 'auto_vulkan', vulkan_available: true, gpu_id: 'auto',
      adapter_index: 1, effective_adapter_index: 1,
      adapters: [
        { gpu_id: id, name: '物理显卡', dxgi_index: 3, vulkan_index: 0 },
        { name: '备用显卡', dxgi_index: 1, vulkan_index: null },
      ],
    }
    const fn = mockFetch({
      'GET /api/detect/config': () => ({ ...CONFIG, runtime }),
      'PUT /api/detect/config': () => {
        runtime.gpu_id = id
        runtime.effective_adapter_index = 3
        return { restarted: 1 }
      },
    })
    const view = mount(DetectPanel)
    await flushPromises()
    const gpuSelect = view.findAll('select').find(select => select.text().includes('物理显卡'))
    await gpuSelect.setValue(id)
    await vi.waitFor(() => expect(view.text()).toContain('DirectML 回退使用同一张显卡'))
    const [, request] = fn.mock.calls.find(([, opts]) => opts?.method === 'PUT')
    expect(JSON.parse(request.body)).toEqual({ runtime: { gpu_id: id } })
    expect(gpuSelect.element.value).toBe(id)
    view.unmount()
  })

  it('三模式卡片渲染且本地模式默认选中', async () => {
    mockFetch({ 'GET /api/detect/config': CONFIG })
    const w = mount(DetectPanel)
    await flushPromises()
    const cards = w.findAll('[role="radio"]')
    expect(cards.length).toBe(3)
    expect(cards[0].attributes('aria-checked')).toBe('true')  // local 默认
    expect(cards[2].text()).toContain('本地 + API 协作')
    expect(w.text()).toContain('今日 API 复核')
  })

  it('点协作卡片即 PUT mode=cascade（即时生效契约）', async () => {
    const fn = mockFetch({
      'GET /api/detect/config': {
        ...CONFIG,
        vlm: { ...CONFIG.vlm, url: 'http://127.0.0.1:8899' }, // 回环已配置 → 允许切换
      },
      'PUT /api/detect/config': { ok: true, restarted: 2 },
    })
    const w = mount(DetectPanel)
    await flushPromises()
    await w.findAll('[role="radio"]')[2].trigger('click')
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT')
    expect(JSON.parse(put[1].body).mode).toBe('cascade')
    expect(w.text()).toContain('2 路跟随全局的相机已重启生效')
  })

  it('远程 URL 未勾选合规确认时拒绝切换 API 模式（前端预检）', async () => {
    const fn = mockFetch({
      'GET /api/detect/config': {
        ...CONFIG,
        vlm: { ...CONFIG.vlm, url: 'https://api.example.com/v1', allow_remote: false },
      },
    })
    const w = mount(DetectPanel)
    await flushPromises()
    await w.findAll('[role="radio"]')[1].trigger('click') // 仅 API
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT')
    expect(put).toBeUndefined()
    expect(w.text()).toContain('合规') // 引导文案
  })

  it('API 档案保存：api_key 留空不上送（不变更语义）', async () => {
    const fn = mockFetch({
      'GET /api/detect/config': {
        ...CONFIG, mode: 'api',
        vlm: { ...CONFIG.vlm, url: 'http://127.0.0.1:8899', api_key_set: true },
      },
      'GET /api/detect/profiles': { active: '', legacy_fallback: true, profiles: [] },
      'POST /api/detect/profiles': { ok: true, active: 'p1', restarted: 0,
        profile: { id: 'p1', name: 'API 服务', provider: 'openai_compat',
                   url: 'http://127.0.0.1:8899', model: '', api_key: '***',
                   api_key_set: true, timeout_s: 30, allow_remote: false } },
    })
    const w = mount(DetectPanel)
    await flushPromises()
    const saveBtn = w.findAll('button').find((b) => b.text() === '保存并启用')
    await saveBtn.trigger('click')
    await flushPromises()
    const post = fn.mock.calls.find((c) => c[1]?.method === 'POST' && c[0].includes('/api/detect/profiles'))
    const body = JSON.parse(post[1].body)
    expect(body.url).toBe('http://127.0.0.1:8899')
    expect(body.api_key).toBeUndefined() // 未输入新值=不变更
    expect(body.activate).toBe(true)
  })

  it('获取模型：POST /api/detect/models 并回填上游模型清单', async () => {
    const fn = mockFetch({
      'GET /api/detect/config': {
        ...CONFIG,
        vlm: { ...CONFIG.vlm, url: 'http://127.0.0.1:8899' },
      },
      'GET /api/detect/profiles': { active: '', legacy_fallback: true, profiles: [] },
      'POST /api/detect/models': { ok: true, elapsed_ms: 12, count: 2,
        models: ['gpt-4o-mini', 'gpt-4o'] },
    })
    const w = mount(DetectPanel)
    await flushPromises()
    const btn = w.findAll('button').find((b) => b.text() === '获取模型')
    await btn.trigger('click')
    await flushPromises()
    const post = fn.mock.calls.find((c) => c[1]?.method === 'POST' && c[0].includes('/api/detect/models'))
    expect(post).toBeTruthy()
    expect(JSON.parse(post[1].body).url).toBe('http://127.0.0.1:8899')
    expect(w.findAll('#upstream-models option').length).toBe(2)
    expect(w.text()).toContain('拉到 2 个模型')
  })

  it('切换已保存档案：POST /profiles/{id}/activate', async () => {
    const fn = mockFetch({
      'GET /api/detect/config': { ...CONFIG, mode: 'api' },
      'GET /api/detect/profiles': {
        active: 'p1', legacy_fallback: false,
        profiles: [
          { id: 'p1', name: '公司 GPT', provider: 'openai_compat', url: 'https://a.example.com',
            model: 'gpt-4o-mini', api_key: '***', api_key_set: true,
            timeout_s: 30, allow_remote: true },
          { id: 'p2', name: '本机 Ollama', provider: 'ollama', url: 'http://127.0.0.1:11434',
            model: 'llava', api_key: '***', api_key_set: false,
            timeout_s: 30, allow_remote: false },
        ],
      },
      'POST /api/detect/profiles/p2/activate': { ok: true, active: 'p2', restarted: 1 },
    })
    const w = mount(DetectPanel)
    await flushPromises()
    const sel = w.find('select[aria-label="已保存的 API 服务"]')
    await sel.setValue('p2')
    await flushPromises()
    const act = w.findAll('button').find((b) => b.text() === '设为当前')
    await act.trigger('click')
    await flushPromises()
    const post = fn.mock.calls.find((c) => c[1]?.method === 'POST' && c[0].includes('/activate'))
    expect(post[0]).toContain('/api/detect/profiles/p2/activate')
    expect(w.text()).toContain('已切换为')
  })

  it('预算耗尽展示降级横幅', async () => {
    mockFetch({
      'GET /api/detect/config': {
        ...CONFIG,
        usage: { date: '2026-11-20', count: 200, budget: 200, exhausted: true },
      },
    })
    const w = mount(DetectPanel)
    await flushPromises()
    expect(w.text()).toContain('今日上限已用尽')
    expect(w.text()).toContain('仅本地')
  })

  it('LS-050：VLM 提示词编辑与保存（留空=默认）', async () => {
    const fn = mockFetch({
      'GET /api/detect/config': {
        ...CONFIG,
        vlm_prompt: '自定义提示词：只数人数',
        perf: { motion_gate_enabled: true, motion_threshold: 2 },
      },
      'PUT /api/detect/config': { ok: true },
    })
    const w = mount(DetectPanel)
    await flushPromises()
    const ta = w.find('textarea[aria-label="VLM 提示词"]')
    expect(ta.exists()).toBe(true)
    expect(ta.element.value).toBe('自定义提示词：只数人数')
    await ta.setValue('新提示词')
    const save = w.findAll('button').find((b) => b.text() === '保存提示词')
    await save.trigger('click')
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT' && JSON.parse(c[1].body).vlm_prompt !== undefined)
    expect(JSON.parse(put[1].body).vlm_prompt).toBe('新提示词')
  })

  it('LS-050：Motion Gate 开关保存 perf 配置', async () => {
    const fn = mockFetch({
      'GET /api/detect/config': {
        ...CONFIG,
        perf: { motion_gate_enabled: false, motion_threshold: 2 },
      },
      'PUT /api/detect/config': { ok: true },
    })
    const w = mount(DetectPanel)
    await flushPromises()
    expect(w.text()).toContain('启用运动预筛')
    const save = w.findAll('button').find((b) => b.text() === '保存性能设置')
    await save.trigger('click')
    await flushPromises()
    const put = fn.mock.calls.find((c) => c[1]?.method === 'PUT' && JSON.parse(c[1].body).perf !== undefined)
    expect(JSON.parse(put[1].body).perf.motion_gate_enabled).toBe(false)
  })
})
