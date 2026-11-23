<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { api, fetchMedia } from '../api'
import { reservePreviewStream } from '../streamPool'
import WebrtcPlayer from './WebrtcPlayer.vue'

const props = defineProps({
  cameraId: { type: Number, required: true },
  cameraType: { type: String, required: true },
  name: { type: String, required: true },
})

const mode = ref('snapshot')
const wsUrl = ref('')
const streamReserved = ref(false)
const playing = ref(false)
const snapshot = ref('')
const capturedAt = ref(0)
const previewEl = ref(null)
let timer
let controller
let generation = 0
let lastProbeAt = 0
let etag = ''
let intersection
let inViewport = typeof IntersectionObserver === 'undefined'
let mounted = false
let releasePreviewStream

watch(wsUrl, (url) => {
  if (url && !releasePreviewStream) {
    releasePreviewStream = reservePreviewStream((granted) => {
      streamReserved.value = granted
    })
  } else if (!url) {
    releasePreviewStream?.()
    releasePreviewStream = undefined
  }
}, { flush: 'sync' })

const snapshotLabel = computed(() => capturedAt.value
  ? `快照 ${new Date(capturedAt.value).toLocaleTimeString('zh-CN', { hour12: false })}`
  : '快照')

function clearSnapshot() {
  if (snapshot.value) URL.revokeObjectURL(snapshot.value)
  snapshot.value = ''
  capturedAt.value = 0
  etag = ''
}

function schedule(delay = 0) {
  clearTimeout(timer)
  if (mounted && !document.hidden && inViewport && !playing.value && !controller) {
    timer = setTimeout(() => poll(generation), delay)
  }
}

async function poll(active) {
  if (active !== generation || document.hidden || !inViewport || playing.value || controller) return
  const request = new AbortController()
  controller = request
  try {
    if (props.cameraType === 'rtsp' && Date.now() - lastProbeAt >= 5000) {
      lastProbeAt = Date.now()
      const preview = await api.cameraWebrtc(props.cameraId, request.signal).catch(() => null)
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
    if (!playing.value && props.cameraType !== 'folder') {
      const response = await fetchMedia(`/api/cameras/${props.cameraId}/snapshot`, request.signal, etag)
      if (response.status === 304) return
      const time = Number(response.headers.get('x-ls-frame-time-ms')) || 0
      const tag = response.headers.get('etag') || ''
      if (!tag && time && time === capturedAt.value) {
        await response.body?.cancel()
        return
      }
      const blob = await response.blob()
      if (active !== generation || playing.value || !blob.size) return
      clearSnapshot()
      capturedAt.value = time
      etag = tag
      snapshot.value = URL.createObjectURL(blob)
    }
  } catch (err) {
    if (active === generation && [401, 403, 404].includes(err?.status)) clearSnapshot()
  } finally {
    if (controller === request) controller = undefined
    if (active === generation && props.cameraType !== 'folder') schedule(1500)
  }
}

function stop() {
  generation++
  controller?.abort()
  controller = undefined
  clearTimeout(timer)
  clearSnapshot()
  wsUrl.value = ''
  playing.value = false
  mode.value = 'snapshot'
  lastProbeAt = 0
}

function restart() {
  stop()
  if (props.cameraType !== 'folder') schedule()
}

function onReady(ready) {
  playing.value = ready
  if (!ready && streamReserved.value) {
    wsUrl.value = ''
    mode.value = 'snapshot'
    lastProbeAt = Date.now()
  }
  schedule()
}

onMounted(() => {
  mounted = true
  if (typeof IntersectionObserver !== 'undefined' && previewEl.value) {
    intersection = new IntersectionObserver(([entry]) => {
      if (entry && entry.isIntersecting !== inViewport) {
        inViewport = entry.isIntersecting
        restart()
      }
    })
    intersection.observe(previewEl.value)
  }
  document.addEventListener('visibilitychange', restart)
  restart()
})
watch(() => [props.cameraId, props.cameraType], restart)
onBeforeUnmount(() => {
  mounted = false
  intersection?.disconnect()
  document.removeEventListener('visibilitychange', restart)
  stop()
})
</script>

<template>
  <div ref="previewEl" class="preview">
    <WebrtcPlayer v-if="wsUrl && streamReserved" class="stream" :class="{ 'stream-pending': !playing }"
      :ws-url="wsUrl" :mode="mode" @ready="onReady" />
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
