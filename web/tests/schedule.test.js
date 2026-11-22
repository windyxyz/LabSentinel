// LS-052：周时段排班纯逻辑 + SchedulePicker 组件契约。
import { describe, expect, it } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import {
  SCHEDULE_PRESETS, describeSchedule, formatDays, isValidHm,
  isOvernight, normalizeWindow, parseDays,
} from '../src/schedule.js'
import SchedulePicker from '../src/components/SchedulePicker.vue'

describe('schedule.js 纯逻辑', () => {
  it('parseDays 支持逗号/区间/空值', () => {
    expect(parseDays('1,2,3')).toEqual([1, 2, 3])
    expect(parseDays('1-5')).toEqual([1, 2, 3, 4, 5])
    expect(parseDays('')).toEqual([])
    expect(parseDays('0,8,abc')).toEqual([])
    expect(parseDays('7,1,1')).toEqual([1, 7])
  })

  it('formatDays 去重排序', () => {
    expect(formatDays([3, 1, 1, 2])).toBe('1,2,3')
    expect(formatDays([])).toBe('')
    expect(formatDays([9, 0])).toBe('')
  })

  it('HH:MM 校验与规范化', () => {
    expect(isValidHm('08:00')).toBe(true)
    expect(isValidHm('23:59')).toBe(true)
    expect(isValidHm('24:00')).toBe(false)
    expect(isValidHm('8:00')).toBe(false)
    expect(normalizeWindow('bad', 'xx')).toEqual({ start: '00:00', end: '23:59' })
  })

  it('跨午夜识别', () => {
    expect(isOvernight('22:00', '08:00')).toBe(true)
    expect(isOvernight('08:00', '18:00')).toBe(false)
  })

  it('摘要文案', () => {
    expect(describeSchedule('1,2,3,4,5,6,7', '00:00', '23:59')).toContain('每天 全天')
    expect(describeSchedule('1,2,3,4,5', '08:00', '18:00')).toContain('周一二三四五 08:00-18:00')
    expect(describeSchedule('1,2,3,4,5,6,7', '22:00', '08:00')).toContain('跨午夜')
  })

  it('快捷模板覆盖四类场景', () => {
    const ids = SCHEDULE_PRESETS.map((p) => p.id)
    expect(ids).toEqual(['all', 'workday', 'night', 'weekend'])
  })
})

describe('SchedulePicker.vue', () => {
  it('应用模板输出后端兼容契约', async () => {
    const w = mount(SchedulePicker, {
      props: { daysOfWeek: '', timeStart: '00:00', timeEnd: '23:59' },
    })
    const night = w.findAll('button').find((b) => b.text().includes('夜间值守'))
    await night.trigger('click')
    await flushPromises()
    const emitted = w.emitted()
    expect(emitted['update:daysOfWeek'].at(-1)).toEqual(['1,2,3,4,5,6,7'])
    expect(emitted['update:timeStart'].at(-1)).toEqual(['22:00'])
    expect(emitted['update:timeEnd'].at(-1)).toEqual(['08:00'])
  })

  it('勾选星期输出逗号分隔', async () => {
    const w = mount(SchedulePicker, {
      props: { daysOfWeek: '1,2', timeStart: '00:00', timeEnd: '23:59' },
    })
    const boxes = w.findAll('input[type="checkbox"]')
    await boxes[2].trigger('change') // 周三
    expect(w.emitted()['update:daysOfWeek'].at(-1)).toEqual(['1,2,3'])
  })
})
