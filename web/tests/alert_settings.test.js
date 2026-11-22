import { afterEach, describe, expect, it, vi } from 'vitest'
import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { api } from '../src/api'
import AlertSettings from '../src/views/AlertSettings.vue'

enableAutoUnmount(afterEach)
afterEach(() => vi.restoreAllMocks())

async function viewWithChannels() {
  vi.spyOn(api, 'alertConfig').mockResolvedValue({ channels: {} })
  vi.spyOn(api, 'pushPolicy').mockResolvedValue({ send_image: false, cooldown_s: 60 })
  const view = mount(AlertSettings)
  await flushPromises()
  return view
}

describe('告警渠道编辑器', () => {
  it('切换渠道和保存只更新相应表单，其他渠道的草稿继续保留', async () => {
    const save = vi.spyOn(api, 'saveAlertConfig').mockResolvedValue({
      channels: { webhook: { configured: true, source: 'db', allow_http: true, has_url: true } },
    })
    const view = await viewWithChannels()
    const select = view.get('.channel-select select')
    await select.setValue('email')
    await view.get('input[placeholder="smtp.qq.com"]').setValue('smtp.example.test')
    await select.setValue('webhook')
    expect(view.findAll('.channel-editor')).toHaveLength(1)
    const url = view.get('.channel-editor input[type="url"]')
    await url.setValue('https://example.test/hook')
    await view.get('.field-toggle input').setValue(true)
    await view.get('.channel-editor').trigger('submit')
    await flushPromises()
    expect(save).toHaveBeenCalledWith({ webhook: { url: 'https://example.test/hook', allow_http: true } })
    expect(view.get('.channel-editor input[type="url"]').element.value).toBe('')
    await select.setValue('email')
    expect(view.get('input[placeholder="smtp.qq.com"]').element.value).toBe('smtp.example.test')
  })

  it('显示失败时的可读反馈并保留渠道测试结果', async () => {
    vi.spyOn(api, 'alertTest').mockResolvedValue({ results: {} })
    const view = await viewWithChannels()
    const editor = view.get('.channel-editor')
    await editor.get('button[type="button"]').trigger('click')
    await flushPromises()
    expect(editor.get('[role="status"]').text()).toContain('服务未返回测试结果')
    expect(view.get('.channel-choice[aria-pressed="true"]').text()).toContain('测试失败')
  })
})
