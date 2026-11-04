// 任务133：AsyncState 三态 + notice + slot + retry 契约（@vue/test-utils 挂载）。
import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import AsyncState from '../src/components/AsyncState.vue'

const slotContent = '<div class="content">正文</div>'

function mountState(props, slots = { default: slotContent }) {
  return mount(AsyncState, { props, slots })
}

describe('AsyncState（任务66/133）', () => {
  it('loading：只显示加载中，不渲染 slot', () => {
    const w = mountState({ loading: true })
    expect(w.text()).toContain('加载中…')
    expect(w.find('.content').exists()).toBe(false)
  })

  it('error：红字 + 重试按钮 + 401 提示 + 发出 retry', async () => {
    const w = mountState({ loading: false, error: 'GET /x -> 401' })
    expect(w.text()).toContain('GET /x -> 401')
    expect(w.text()).toContain('填写管理令牌')
    await w.find('button').trigger('click')
    expect(w.emitted('retry')).toHaveLength(1)
  })

  it('error 非 401 无令牌提示', () => {
    const w = mountState({ loading: false, error: 'boom' })
    expect(w.text()).not.toContain('填写管理令牌')
  })

  it('empty：显示 emptyText，不渲染 slot', () => {
    const w = mountState({ loading: false, empty: true, emptyText: '暂无插件' })
    expect(w.text()).toContain('暂无插件')
    expect(w.find('.content').exists()).toBe(false)
  })

  it('正常：渲染 slot；notice 顶部轻提示并存', () => {
    const w = mountState({ loading: false, notice: '刷新失败，仍显示上一次数据' })
    expect(w.find('.content').exists()).toBe(true)
    expect(w.text()).toContain('刷新失败，仍显示上一次数据')
  })
})
