<!-- LS-051：实时检测框 / 跟踪 ID / 合规态 / 安全区 Canvas 叠加层。
     零视频重编码：只订阅 /ws/detections/:cid 元数据，按容器等比缩放绘制。 -->
<template>
  <canvas v-show="visible" ref="canvasEl" class="video-overlay" aria-hidden="true"></canvas>
</template>

<script setup>
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { requestToken } from '../api'
import { mapBox, stateColor } from '../overlay'

const props = defineProps({
  cameraId: { type: [Number, String], required: true },
  imgW: { type: Number, default: 1280 },
  imgH: { type: Number, default: 720 },
  visible: { type: Boolean, default: true },
})

const canvasEl = ref(null)
const emit = defineEmits(['tracks'])
let ws = null
let reconnectTimer = null
let closed = false
let latest = null
let raf = 0

function connect() {
  if (closed) return
  const proto = location.protocol === 'https:' ? 'wss' : 'ws'
  const token = requestToken()
  // 令牌经子协议传递（不落 URL/日志，与 /ws/events 同口径）
  const protocols = token ? ['labsentinel', 'bearer.' + token] : ['labsentinel']
  try {
    ws = new WebSocket(`${proto}://${location.host}/ws/detections/${props.cameraId}`, protocols)
  } catch {
    scheduleReconnect()
    return
  }
  ws.onmessage = (m) => {
    try {
      const d = JSON.parse(m.data)
      if (String(d?.camera_id) === String(props.cameraId)) {
        latest = d
        emit('tracks', Array.isArray(d.tracks) ? d.tracks.length : 0)
      }
    } catch { /* 忽略坏帧 */ }
  }
  ws.onclose = () => { ws = null; latest = null; emit('tracks', 0); scheduleReconnect() }
  ws.onerror = () => { try { ws?.close() } catch { /* ignore */ } }
}

function scheduleReconnect() {
  if (closed) return
  clearTimeout(reconnectTimer)
  reconnectTimer = setTimeout(connect, 3000)
}

function draw() {
  const c = canvasEl.value
  if (!c) return
  const parent = c.parentElement
  const dispW = parent?.clientWidth || 0
  const dispH = parent?.clientHeight || 0
  if (c.width !== dispW || c.height !== dispH) {
    c.width = dispW
    c.height = dispH
  }
  const ctx = c.getContext('2d')
  if (!ctx) return // 无 2D 上下文（jsdom/极简浏览器）——静默降级
  ctx.clearRect(0, 0, c.width, c.height)
  if (!latest || !props.visible) return
  const iw = latest.img_w || props.imgW
  const ih = latest.img_h || props.imgH
  // 安全区多边形（蓝色半透明）
  for (const z of Array.isArray(latest.zones) ? latest.zones : []) {
    const points = Array.isArray(z?.points) ? z.points.filter(Array.isArray) : []
    const pts = points.map(([x, y]) => mapBox([x, y, x, y], iw, ih, dispW, dispH))
    const valid = pts.filter(Boolean)
    if (valid.length < 3) continue
    ctx.beginPath()
    valid.forEach((p, i) => (i ? ctx.lineTo(p.x, p.y) : ctx.moveTo(p.x, p.y)))
    ctx.closePath()
    ctx.fillStyle = 'rgba(77,163,255,0.10)'
    ctx.fill()
    ctx.strokeStyle = 'rgba(77,163,255,0.85)'
    ctx.lineWidth = 2
    ctx.stroke()
  }
  // 人框 + 跟踪 ID + 合规态颜色
  ctx.font = '13px ui-monospace, monospace'
  for (const t of Array.isArray(latest.tracks) ? latest.tracks : []) {
    const r = mapBox(t?.box, iw, ih, dispW, dispH)
    if (!r) continue
    const color = stateColor(t.ppe_state)
    ctx.strokeStyle = color
    ctx.lineWidth = 2
    ctx.strokeRect(r.x, r.y, r.w, r.h)
    const label = `#${t.id}`
    const tw = ctx.measureText(label).width + 8
    ctx.fillStyle = color
    ctx.fillRect(r.x, Math.max(0, r.y - 18), tw, 18)
    ctx.fillStyle = '#0b0f14'
    ctx.fillText(label, r.x + 4, Math.max(12, r.y - 5))
  }
}

function loop() {
  draw()
  raf = requestAnimationFrame(loop)
}

function closeSocket() {
  clearTimeout(reconnectTimer)
  if (ws) {
    ws.onopen = ws.onmessage = ws.onclose = ws.onerror = null
    try { ws.close() } catch { /* ignore */ }
    ws = null
  }
}

watch(() => props.cameraId, () => {
  closeSocket()
  latest = null
  connect()
})

onMounted(() => {
  connect()
  raf = requestAnimationFrame(loop)
})

onBeforeUnmount(() => {
  closed = true
  closeSocket()
  cancelAnimationFrame(raf)
  clearTimeout(reconnectTimer)
  try { ws?.close() } catch { /* ignore */ }
})
</script>

<style scoped>
.video-overlay {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  pointer-events: none;
  z-index: 2;
}
</style>
