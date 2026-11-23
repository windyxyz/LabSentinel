<script setup>
import { computed, ref, watch } from 'vue'
import { api } from '../api'

const props = defineProps({
  eid: { type: Number, required: true },
  src: { type: String, required: true },
  available: { type: Boolean, default: false },
})
const emit = defineEmits(['zoom', 'error'])
const overlay = ref(null)
const visible = ref(false)
const busy = ref(false)
const error = ref('')
watch(() => props.eid, () => {
  overlay.value = null
  visible.value = false
  busy.value = false
  error.value = ''
})

const finite = (v) => typeof v === 'number' && Number.isFinite(v)
const box = computed(() => {
  const data = overlay.value?.box
  if (!Array.isArray(data) || data.length !== 4 || !data.every(finite)) return null
  const [x, y, right, bottom] = data
  return right > x && bottom > y ? { x, y, width: right - x, height: bottom - y } : null
})
const polygon = computed(() => {
  const data = overlay.value?.zone
  if (!Array.isArray(data)) return ''
  const points = data.filter(point => Array.isArray(point) && point.length === 2 && point.every(finite))
  return points.length >= 3 ? points.map(point => point.join(',')).join(' ') : ''
})

async function toggle() {
  if (overlay.value) {
    visible.value = !visible.value
    return
  }
  const id = props.eid
  busy.value = true
  error.value = ''
  try {
    const result = await api.eventOverlay(id)
    if (id !== props.eid) return
    if (!finite(result?.width) || !finite(result?.height) || result.width <= 0 || result.height <= 0) {
      error.value = '该截图的叠框已不可用。'
      return
    }
    overlay.value = result
    visible.value = true
  } catch (failure) {
    if (id === props.eid) error.value = failure.message
  } finally {
    if (id === props.eid) busy.value = false
  }
}
</script>

<template>
  <div class="snapshot">
    <button class="snapshot-image" type="button" aria-label="放大事件原图" @click="emit('zoom', src)">
      <img class="drawer-snap" :src="src" alt="事件截图" @error="emit('error')" />
      <svg v-if="overlay && visible" class="overlay" :viewBox="`0 0 ${overlay.width} ${overlay.height}`" aria-hidden="true">
        <polygon v-if="polygon" :points="polygon" />
        <rect v-if="box" v-bind="box" />
      </svg>
    </button>
    <div v-if="available" class="row">
      <button type="button" class="ghost" :aria-pressed="visible" :disabled="busy" @click="toggle">
        {{ visible ? '隐藏检测框' : '显示检测框' }}
      </button>
      <span v-if="error" role="status">{{ error }}</span>
    </div>
  </div>
</template>

<style scoped>
.snapshot { margin-block: 8px; }
.snapshot-image { position: relative; display: block; overflow: hidden; width: 100%; padding: 0; border: none; background: transparent; cursor: zoom-in; border-radius: var(--overlay-radius); }
img { display: block; width: 100%; height: auto; }
.overlay { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
.overlay polygon { fill: #38bdf820; stroke: #38bdf8; stroke-width: 3; }
.overlay rect { fill: none; stroke: #facc15; stroke-width: 4; }
.snapshot .row { margin-top: 8px; }
</style>
