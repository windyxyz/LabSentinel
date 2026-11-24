import { afterEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import SafetyAssistant from '../src/views/SafetyAssistant.vue'

afterEach(() => vi.unstubAllGlobals())

describe('AI 安全助手', () => {
  it('从事件带入机位与问题，仅允许首轮附图', async () => {
    const asked = []
    const session = { id: 4, title: '追问', revision: 0, scope: { camera_id: 9 } }
    vi.stubGlobal('fetch', vi.fn(async (url, options) => {
      const path = url.split('?')[0]
      let result
      if (path === '/api/cameras') result = [{ id: 9, name: '夜视机位' }]
      else if (path === '/api/assistant/sessions') {
        result = options.method === 'POST' ? session : []
      } else if (path === '/api/assistant/sessions/4/messages') {
        const body = JSON.parse(options.body)
        asked.push(body)
        result = { ...body, revision: body.revision + 1, answer: '设备门开启。', evidence_mode: body.include_images ? 'images' : 'record_only', model: 'fixture', facts: { event: { id: 23 } } }
      } else throw new Error(path)
      return { ok: true, json: async () => result }
    }))
    const wrapper = mount(SafetyAssistant, { props: { routeQuery: 'event=23&camera=9' } })
    await flushPromises()
    expect(wrapper.get('textarea').element.value).toContain('#23')
    expect(wrapper.get('select').element.value).toBe('9')
    await wrapper.get('input[type=checkbox]').setValue(true)
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(asked[0]).toMatchObject({ include_images: true, revision: 0 })
    expect(wrapper.find('input[type=checkbox]').exists()).toBe(false)
    await wrapper.get('textarea').setValue('复核结果是什么？')
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(asked[1]).toMatchObject({ include_images: false, revision: 1 })
    expect(wrapper.findAll('a').map(link => link.attributes('href'))).toContain('#/events?event=23')
    wrapper.unmount()
  })
})
