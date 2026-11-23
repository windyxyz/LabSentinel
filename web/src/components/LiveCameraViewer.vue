<script setup>
import { computed, ref, watch } from 'vue'
import CameraPreview from './CameraPreview.vue'
import VideoOverlayCanvas from './VideoOverlayCanvas.vue'

const props = defineProps({
  camera: { type: Object, required: true },
})
const zoom = ref(1)
const center = ref({ x: 0.5, y: 0.5 })
const debug = ref(false)
const tracks = ref(0)
let drag = null
function clampCenter(x, y) {
  const half = 0.5 / zoom.value
  const clamp = (value) => Math.max(half, Math.min(1 - half, value))
  center.value = { x: clamp(x), y: clamp(y) }
}
function reset() {
  zoom.value = 1
  clampCenter(0.5, 0.5)
  drag = null
}
watch(() => props.camera.id, reset)
watch(zoom, () => clampCenter(center.value.x, center.value.y))
const transform = computed(() => {
  const { x, y } = center.value
  return `translate(${(0.5 - x) * zoom.value * 100}%, ${(0.5 - y) * zoom.value * 100}%) scale(${zoom.value})`
})
const minimap = computed(() => {
  const fraction = 1 / zoom.value
  return {
    width: `${fraction * 100}%`, height: `${fraction * 100}%`,
    left: `${(center.value.x - fraction / 2) * 100}%`,
    top: `${(center.value.y - fraction / 2) * 100}%`,
  }
})
function startDrag(event) {
  if (zoom.value <= 1 || event.button !== 0) return
  const { width, height } = event.currentTarget.getBoundingClientRect()
  drag = { id: event.pointerId, x: event.clientX, y: event.clientY, width, height, center: center.value }
  event.currentTarget.setPointerCapture?.(event.pointerId)
}
function moveDrag(event) {
  if (!drag || drag.id !== event.pointerId) return
  clampCenter(drag.center.x - (event.clientX - drag.x) / (drag.width * zoom.value),
    drag.center.y - (event.clientY - drag.y) / (drag.height * zoom.value))
}
function panKey(event) {
  const deltas = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }
  if (zoom.value <= 1 || !deltas[event.key]) return
  event.preventDefault()
  const [x, y] = deltas[event.key]
  clampCenter(center.value.x + x * 0.08 / zoom.value, center.value.y + y * 0.08 / zoom.value)
}
function jump(event) {
  const bounds = event.currentTarget.getBoundingClientRect()
  if (event.detail === 0) clampCenter(0.5, 0.5)
  else clampCenter((event.clientX - bounds.left) / bounds.width, (event.clientY - bounds.top) / bounds.height)
}
</script>

<template>
  <div class="viewer">
    <div class="viewport">
      <div class="scene" :style="{ transform }">
        <CameraPreview :camera-id="camera.id" :camera-type="camera.type" :name="camera.name" />
        <VideoOverlayCanvas v-if="camera.type !== 'folder'" :camera-id="camera.id" :debug="debug" @tracks="count => tracks = count" />
      </div>
      <div v-if="zoom > 1" class="pan-area" tabindex="0" aria-label="方向键或拖动平移放大的图像"
           @pointerdown="startDrag" @pointermove="moveDrag" @pointerup="drag = null"
           @pointercancel="drag = null" @lostpointercapture="drag = null" @keydown="panKey"></div>
      <span v-if="debug" class="track-count">跟踪目标 {{ tracks }}</span>
      <button v-if="zoom > 1" class="minimap" type="button" aria-label="缩放位置，点击跳转" @click="jump">
        <span :style="minimap"></span>
      </button>
    </div>
    <div v-if="camera.type !== 'folder'" class="viewer-controls row">
      <label>数字缩放
        <input v-model.number="zoom" type="range" min="1" max="5" step="0.25" />
        <output>{{ zoom }}×</output>
      </label>
      <button type="button" class="ghost" :disabled="zoom === 1" @click="reset">重置视图</button>
      <button type="button" class="ghost" :aria-pressed="debug" @click="debug = !debug">{{ debug ? '隐藏调试' : '检测调试' }}</button>
    </div>
  </div>
</template>

<style scoped>
.viewport { position: relative; height: min(65vh, 54vw); overflow: hidden; background: var(--bg-base); border-radius: var(--overlay-radius); }
.scene, .pan-area { position: absolute; inset: 0; }
.pan-area { cursor: grab; touch-action: none; }
.pan-area:active { cursor: grabbing; }
.viewer-controls { padding: 8px; background: var(--bg-base); }
.viewer-controls label { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
.minimap { position: absolute; right: 8px; bottom: 8px; width: 88px; height: 50px; padding: 0; background: #111827d9; border: 1px solid #e2e8f0; }
.minimap span { position: absolute; border: 2px solid #fde047; background: #fde04726; pointer-events: none; box-sizing: border-box; }
.track-count { position: absolute; top: 8px; left: 8px; padding: 3px 7px; background: #111827d9; color: white; border-radius: 4px; }
</style>
