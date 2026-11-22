// LS-051：检测框叠加的纯函数（无 DOM 依赖，便于单测）。
// 源图坐标 → 容器显示坐标：等比 letterbox（居中留边）。
export function mapBox(box, imgW, imgH, dispW, dispH) {
  if (!Array.isArray(box) || box.length < 4) return null
  const [x1, y1, x2, y2] = box
  if (![x1, y1, x2, y2].every((n) => Number.isFinite(n))) return null
  if (!(imgW > 0 && imgH > 0 && dispW > 0 && dispH > 0)) return null
  const scale = Math.min(dispW / imgW, dispH / imgH)
  const offX = (dispW - imgW * scale) / 2
  const offY = (dispH - imgH * scale) / 2
  return {
    x: x1 * scale + offX,
    y: y1 * scale + offY,
    w: Math.max(1, (x2 - x1) * scale),
    h: Math.max(1, (y2 - y1) * scale),
    scale,
    offX,
    offY,
  }
}

/** 合规态 → 颜色（绿=合规 / 红=违规 / 灰=待定）。 */
export function stateColor(state) {
  if (state === 'compliant') return '#22c55e'
  if (state === 'violation') return '#ef4444'
  return '#9ca3af'
}
