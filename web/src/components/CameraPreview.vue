<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { api, fetchMedia } from '../api'
import WebrtcPlayer from './WebrtcPlayer.vue'

const props = defineProps({
  cameraId: { type: Number, required: true },
  cameraType: { type: String, required: true },
  name: { type: String, required: true },
})

const mode = ref('snapshot')
const wsUrl = ref('')
const playing = ref(false)
const snapshot = ref('')
const capturedAt = ref(0)
let timer
let controller
let generation = 0
let lastProbeAt = 0

const snapshotLabel = computed(() => capturedAt.value
  ? `快照 ${new Date(capturedAt.value).toLocaleTimeString('zh-CN', { hour12: false })}`
  : '快照')

function clearSnapshot() {
  if (snapshot.value) URL.revokeObjectURL(snapshot.value)
  snapshot.value = ''
  capturedAt.value = 0
}

async function poll(active) {
  if (active !== generation) return
  try {
    if (!document.hidden && props.cameraType === 'rtsp' && !playing.value && Date.now() - lastProbeAt >= 5000) {
      lastProbeAt = Date.now()
      const preview = await api.cameraWebrtc(props.cameraId).catch(() => null)
      if (active !== generation) return
      if (['webrtc', 'mse'].includes(preview?.mode) && typeof preview?.ws_url === 'string') {
        mode.value = preview.mode
        wsUrl.value = preview.ws_url
      } else {
        playing.value = false
        wsUrl.value = ''
        mode.value = 'snapshot'
      }
    }
    if (!document.hidden && !playing.value && props.cameraType !== 'folder') {
      controller = new AbortController()
      const response = await fetchMedia(`/api/cameras/${props.cameraId}/snapshot`, controller.signal)
      const time = Number(response.headers.get('x-ls-frame-time-ms')) || 0
      if (time && time === capturedAt.value) {
        await response.body?.cancel()
        return
      }
      const blob = await response.blob()
      if (active !== generation || playing.value || !blob.size) return
      clearSnapshot()
      capturedAt.value = time
      snapshot.value = URL.createObjectURL(blob)
    }
  } catch (err) {
    if (active === generation && [401, 403, 404].includes(err?.status)) clearSnapshot()
  } finally {
    if (active === generation) timer = setTimeout(() => poll(active), playing.value ? 3000 : 1500)
  }
}

function stop() {
  generation++
  controller?.abort()
  clearTimeout(timer)
  clearSnapshot()
}

function restart() {
  stop()
  wsUrl.value = ''
  playing.value = false
  mode.value = 'snapshot'
  lastProbeAt = 0
  poll(generation)
}

onMounted(restart)
watch(() => [props.cameraId, props.cameraType], restart)
onBeforeUnmount(stop)
</script>

<template>
  <div class="preview">
    <WebrtcPlayer v-if="wsUrl" class="stream" :class="{ 'stream-pending': !playing }"
      :ws-url="wsUrl" :mode="mode" @ready="playing = $event" />
    <template v-if="!playing">
      <img v-if="snapshot" :src="snapshot" :alt="name" />
      <div v-else class="placeholder">{{ cameraType === 'folder' ? '图片文件夹' : '等待画面' }}</div>
    </template>
    <span v-if="snapshot && !playing" class="snapshot-time">{{ snapshotLabel }}</span>
  </div>
</template>

<style scoped>
.preview { position: relative; width: 100%; height: 100%; background: var(--bg-base); }
.stream, .preview img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; }
.stream-pending { opacity: 0; }
.placeholder { position: absolute; inset: 0; display: grid; place-items: center; color: var(--text-secondary); }
.snapshot-time { position: absolute; bottom: 8px; left: 8px; padding: 2px 6px; border-radius: 4px; background: var(--bg-base); color: var(--text-primary); font-size: 12px; }
</style>
