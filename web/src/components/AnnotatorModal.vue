<!-- LS-054：样本标注弹窗——拉框 / 选中删除 / 切类别 / 标记核验。
     坐标以样本原图像素存储（导出时归一化为 YOLO txt）。 -->
<template>
  <Teleport to="body">
    <div v-if="sample" class="annotator-mask" @click.self="$emit('close')">
      <div class="annotator">
        <header class="row" style="align-items:center;justify-content:space-between">
          <b>标注样本 #{{ sample.id }}（{{ sample.source_type }}）</b>
          <button class="ghost" @click="$emit('close')">关闭</button>
        </header>
        <div ref="viewport" class="viewport" @wheel.prevent="onWheel">
          <div class="canvas-wrap" ref="wrapEl" :style="{ width: `${zoom * 100}%` }">
            <img v-if="image" :src="image" alt="待标注样本" @load="onImgLoad" />
            <canvas ref="canvasEl" @pointerdown="onDown" @pointermove="onMove" @pointerup="onUp"
                    @pointercancel="cancelDrag"></canvas>
          </div>
        </div>
        <div class="row" style="align-items:center;flex-wrap:wrap;gap:8px;margin-top:8px">
          <select v-model="cls" aria-label="标注类别">
            <option v-for="c in CLASSES" :key="c" :value="c">{{ c }}</option>
          </select>
          <span class="muted">拖拽画框或调整角点 · 滚轮缩放 · Alt + 拖拽平移 · 数字键切类</span>
          <button class="ghost" :disabled="selected < 0" @click="deleteSelected">删除选中框</button>
          <span class="muted">已标注 {{ boxes.length }} 个</span>
        </div>
        <div class="row" style="align-items:center;gap:8px;margin-top:8px">
          <button :disabled="busy" @click="save('verified')">保存并标记已核验</button>
          <button class="ghost" :disabled="busy" @click="save('ambiguous')">标记模糊弃用</button>
          <button class="ghost" :disabled="busy" @click="save(sample.status || 'pending')">仅保存</button>
          <span v-if="msg" class="muted">{{ msg }}</span>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<script setup>
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { api, fetchBinary } from '../api'
import { imagePoint } from '../overlay'

const props = defineProps({
  sample: { type: Object, default: null },
  imageUrl: { type: String, default: '' },
})
const emit = defineEmits(['close', 'saved'])

const CLASSES = ['person', 'lab_coat', 'no_lab_coat', 'gloves', 'no_gloves', 'goggles', 'no_goggles', 'mask']
const wrapEl = ref(null)
const viewport = ref(null)
const image = ref('')
const zoom = ref(1)
const canvasEl = ref(null)
const boxes = ref([])
const cls = ref('person')
const selected = ref(-1)
const busy = ref(false)
const msg = ref('')
let dragging = null
let panning = null
let imageRequest = 0

watch(() => props.sample, (s) => {
  boxes.value = Array.isArray(s?.boxes) ? JSON.parse(JSON.stringify(s.boxes)) : []
  selected.value = -1
  msg.value = ''
  zoom.value = 1
  dragging = null
  panning = null
}, { immediate: true })

watch(() => props.imageUrl, async (path) => {
  const request = ++imageRequest
  if (image.value) URL.revokeObjectURL(image.value)
  image.value = ''
  if (!path) return
  try {
    const blob = await fetchBinary(path)
    if (request === imageRequest) image.value = URL.createObjectURL(blob)
  } catch (err) {
    if (request === imageRequest) msg.value = err.message
  }
}, { immediate: true })

function imgEl() {
  return wrapEl.value?.querySelector('img')
}
function scale() {
  const img = imgEl()
  if (!img || !props.sample) return { sx: 1, sy: 1, dw: 0, dh: 0 }
  const dw = img.clientWidth
  const dh = img.clientHeight
  return {
    sx: dw ? props.sample.width / dw : 1,
    sy: dh ? props.sample.height / dh : 1,
    dw, dh,
  }
}
function pos(e) {
  return imagePoint(e.clientX, e.clientY, imgEl().getBoundingClientRect(), props.sample.width, props.sample.height)
}
function onImgLoad() { draw() }
function color(name) { return getComputedStyle(canvasEl.value).getPropertyValue(name).trim() }
function draw() {
  const c = canvasEl.value
  const img = imgEl()
  if (!c || !img) return
  c.width = img.clientWidth
  c.height = img.clientHeight
  const ctx = c.getContext('2d')
  if (!ctx) return
  ctx.clearRect(0, 0, c.width, c.height)
  const { sx, sy } = scale()
  boxes.value.forEach((b, i) => {
    const x = b.x1 / sx
    const y = b.y1 / sy
    const w = (b.x2 - b.x1) / sx
    const h = (b.y2 - b.y1) / sy
    ctx.strokeStyle = color(i === selected.value ? '--l2' : '--accent-strong')
    ctx.lineWidth = i === selected.value ? 3 : 2
    ctx.strokeRect(x, y, w, h)
    if (i === selected.value) {
      ctx.fillStyle = ctx.strokeStyle
      for (const [px, py] of [[x, y], [x + w, y], [x, y + h], [x + w, y + h]]) {
        ctx.fillRect(px - 3, py - 3, 6, 6)
      }
    }
    ctx.fillStyle = 'rgba(0,0,0,.55)'
    const label = b.class || 'person'
    ctx.font = '12px sans-serif'
    const tw = ctx.measureText(label).width + 8
    ctx.fillRect(x, Math.max(0, y - 16), tw, 16)
    ctx.fillStyle = '#fff'
    ctx.fillText(label, x + 4, Math.max(11, y - 4))
  })
  if (dragging) {
    const { sx: s2, sy: s3 } = scale()
    ctx.strokeStyle = color('--l2')
    ctx.setLineDash([4, 3])
    ctx.strokeRect(dragging.x1 / s2, dragging.y1 / s3,
      (dragging.x2 - dragging.x1) / s2, (dragging.y2 - dragging.y1) / s3)
    ctx.setLineDash([])
  }
}
function onDown(e) {
  if (!props.sample || !imgEl() || (e.button !== 0 && e.button !== 1)) return
  e.preventDefault()
  e.currentTarget.setPointerCapture?.(e.pointerId)
  if (e.altKey || e.button === 1) {
    panning = { x: e.clientX, y: e.clientY, left: viewport.value.scrollLeft, top: viewport.value.scrollTop }
    return
  }
  const p = pos(e)
  const current = boxes.value[selected.value]
  if (current) {
    const { sx, sy } = scale()
    const corner = [
      ['x1', 'y1', 'x2', 'y2'], ['x1', 'y2', 'x2', 'y1'],
      ['x2', 'y1', 'x1', 'y2'], ['x2', 'y2', 'x1', 'y1'],
    ].find(([x, y]) => Math.hypot((current[x] - p.x) / sx, (current[y] - p.y) / sy) <= 10)
    if (corner) {
      const [x, y, otherX, otherY] = corner
      dragging = { x1: current[otherX], y1: current[otherY], x2: current[x], y2: current[y], edit: selected.value }
      return
    }
  }
  dragging = { x1: p.x, y1: p.y, x2: p.x, y2: p.y }
}
function onMove(e) {
  if (panning) {
    viewport.value.scrollLeft = panning.left - (e.clientX - panning.x)
    viewport.value.scrollTop = panning.top - (e.clientY - panning.y)
    return
  }
  if (!dragging) return
  const p = pos(e)
  dragging.x2 = p.x
  dragging.y2 = p.y
  draw()
}
function onUp(e) {
  if (panning) { panning = null; return }
  if (!dragging) return
  const x1 = Math.min(dragging.x1, dragging.x2)
  const y1 = Math.min(dragging.y1, dragging.y2)
  const x2 = Math.max(dragging.x1, dragging.x2)
  const y2 = Math.max(dragging.y1, dragging.y2)
  const { edit } = dragging
  dragging = null
  if (x2 - x1 >= 2 && y2 - y1 >= 2) {
    const rect = { x1, y1, x2, y2 }
    if (edit === undefined) {
      boxes.value.push({ class: cls.value, ...rect })
      selected.value = boxes.value.length - 1
    } else {
      Object.assign(boxes.value[edit], rect)
    }
  } else if (edit === undefined) {
    onClick(e)
  }
  draw()
}
function cancelDrag() {
  panning = null
  dragging = null
  draw()
}
async function onWheel(e) {
  if (!imgEl()) return
  const el = viewport.value
  const r = el.getBoundingClientRect()
  const x = e.clientX - r.left
  const y = e.clientY - r.top
  const previous = zoom.value
  const left = el.scrollLeft
  const top = el.scrollTop
  zoom.value = Math.max(1, Math.min(5, previous * (e.deltaY < 0 ? 1.15 : 1 / 1.15)))
  await nextTick()
  const ratio = zoom.value / previous
  el.scrollLeft = (x + left) * ratio - x
  el.scrollTop = (y + top) * ratio - y
  draw()
}
function onClick(e) {
  if (dragging) return
  const p = pos(e)
  selected.value = -1
  // 命中测试：从后往前（后画的在上层）
  for (let i = boxes.value.length - 1; i >= 0; i--) {
    const b = boxes.value[i]
    if (p.x >= b.x1 && p.x <= b.x2 && p.y >= b.y1 && p.y <= b.y2) {
      selected.value = i
      break
    }
  }
  draw()
}
function deleteSelected() {
  if (selected.value < 0) return
  boxes.value.splice(selected.value, 1)
  selected.value = -1
  draw()
}
function onKey(e) {
  if (!props.sample || e.target?.matches?.('input, textarea, select, button, [contenteditable]')) return
  const className = CLASSES[Number(e.key) - 1]
  if (/^[1-8]$/.test(e.key) && className) {
    cls.value = className
    if (selected.value >= 0) boxes.value[selected.value].class = className
    draw()
  }
  if (e.key === 'Enter') { e.preventDefault(); save('verified', true) }
  if (e.key === 'Delete' || e.key === 'Backspace') {
    if (selected.value >= 0) { e.preventDefault(); deleteSelected() }
  }
}
async function save(status, next = false) {
  if (!props.sample || busy.value) return
  busy.value = true
  msg.value = ''
  try {
    await api.datasetUpdate(props.sample.id, { boxes: boxes.value, status })
    msg.value = '已保存'
    emit('saved', { id: props.sample.id, status, next })
  } catch (e) { msg.value = String(e.message || e) }
  busy.value = false
}
onMounted(() => {
  window.addEventListener('keydown', onKey)
  window.addEventListener('resize', draw)
})
onBeforeUnmount(() => {
  imageRequest++
  if (image.value) URL.revokeObjectURL(image.value)
  window.removeEventListener('keydown', onKey)
  window.removeEventListener('resize', draw)
})
</script>

<style scoped>
.annotator-mask { position: fixed; inset: 0; background: rgba(0,0,0,.6); z-index: 70;
  display: flex; align-items: center; justify-content: center; }
.annotator { background: var(--dialog-bg); color: var(--text-primary);
  border: 1px solid var(--stroke-default); border-radius: var(--overlay-radius);
  box-shadow: var(--shadow-dialog); padding: 14px; width: min(900px, 94vw); max-height: 92vh; overflow: auto; }
.viewport { margin-top: 8px; max-height: 62vh; overflow: auto; }
.canvas-wrap { position: relative; }
.canvas-wrap img { width: 100%; display: block; border-radius: 6px; }
.canvas-wrap canvas { position: absolute; inset: 0; width: 100%; height: 100%; cursor: crosshair; touch-action: none; }
</style>
