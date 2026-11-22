// LS-052：可视化周时段排班的纯逻辑（无 DOM，便于单测）。
// 输出严格兼容后端契约：days_of_week = "1,2,3,4,5,6,7"（ISO 周一=1），
// time_start / time_end = "HH:MM"（支持跨午夜，如 22:00-08:00）。

export const WEEKDAY_LABELS = ['一', '二', '三', '四', '五', '六', '日']

/** 快捷模板。 */
export const SCHEDULE_PRESETS = [
  { id: 'all', label: '全天 24h', days: '1,2,3,4,5,6,7', start: '00:00', end: '23:59' },
  { id: 'workday', label: '工作日白天 08:00-18:00', days: '1,2,3,4,5', start: '08:00', end: '18:00' },
  { id: 'night', label: '夜间值守 22:00-08:00', days: '1,2,3,4,5,6,7', start: '22:00', end: '08:00' },
  { id: 'weekend', label: '仅周末', days: '6,7', start: '00:00', end: '23:59' },
]

/** 解析 "1,2,3" / "1-5" / "" → 去重排序后的数字数组（1..7）。 */
export function parseDays(raw) {
  const set = new Set()
  const s = String(raw || '').trim()
  if (!s) return []
  for (const part of s.split(',')) {
    const p = part.trim()
    if (!p) continue
    if (p.includes('-')) {
      const [a, b] = p.split('-').map((x) => Number(x.trim()))
      if (Number.isInteger(a) && Number.isInteger(b)) {
        for (let d = Math.min(a, b); d <= Math.max(a, b); d++) {
          if (d >= 1 && d <= 7) set.add(d)
        }
      }
    } else {
      const d = Number(p)
      if (Number.isInteger(d) && d >= 1 && d <= 7) set.add(d)
    }
  }
  return [...set].sort((a, b) => a - b)
}

/** 数字数组 → "1,2,3"（空数组 → "" = 全部）。 */
export function formatDays(days) {
  const uniq = [...new Set((days || []).filter((d) => d >= 1 && d <= 7))].sort((a, b) => a - b)
  return uniq.join(',')
}

/** 校验 HH:MM。 */
export function isValidHm(v) {
  return /^([01]\d|2[0-3]):[0-5]\d$/.test(String(v || '').trim())
}

/** 规范化时段：非法输入回退 00:00 / 23:59。 */
export function normalizeWindow(start, end) {
  return {
    start: isValidHm(start) ? start.trim() : '00:00',
    end: isValidHm(end) ? end.trim() : '23:59',
  }
}

/** 是否跨午夜（start > end）。 */
export function isOvernight(start, end) {
  const { start: s, end: e } = normalizeWindow(start, end)
  return s > e
}

/** 周时段摘要文案（列表展示）。 */
export function describeSchedule(daysRaw, start, end) {
  const days = parseDays(daysRaw)
  const { start: s, end: e } = normalizeWindow(start, end)
  const dayText = days.length === 7 ? '每天' : days.length === 0 ? '每天' : '周' + days.map((d) => WEEKDAY_LABELS[d - 1]).join('')
  const overnight = s > e ? '（跨午夜）' : ''
  if (s === '00:00' && (e === '23:59' || e === '24:00')) return `${dayText} 全天`
  return `${dayText} ${s}-${e}${overnight}`
}
