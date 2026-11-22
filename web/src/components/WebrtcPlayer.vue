<script setup>
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { requestToken } from '../api'

const props = defineProps({
  wsUrl: { type: String, required: true },
  mode: { type: String, default: 'webrtc' },
})
const emit = defineEmits(['ready'])
const video = ref(null)
const FRAME_TIMEOUT_MS = 12000
const MAX_MSE_QUEUE_BYTES = 4 * 1024 * 1024
let session = null
let retry = null
let attempts = 0
let mounted = false

function disconnect() {
  const old = session
  session = null
  if (!old) return
  clearTimeout(old.watchdog)
  clearTimeout(old.iceTimer)
  old.ws.onopen = old.ws.onmessage = old.ws.onclose = null
  old.ws.close()
  old.pc?.close()
  if (old.objectUrl) URL.revokeObjectURL(old.objectUrl)
  if (video.value) {
    video.value.srcObject = null
    video.value.removeAttribute('src')
  }
  emit('ready', false)
}

function fail(current) {
  if (!mounted || session !== current) return
  disconnect()
  const delay = Math.min(15000, 500 * (2 ** Math.min(attempts++, 5)))
  retry = setTimeout(connect, delay)
}

function observeFrames() {
  const current = session
  if (!current || !video.value || video.value.readyState < 2) return
  clearTimeout(current.watchdog)
  current.watchdog = setTimeout(() => fail(current), FRAME_TIMEOUT_MS)
  if (!current.ready) {
    current.ready = true
    attempts = 0
    emit('ready', true)
  }
}

function send(current, type, value) {
  if (session === current && current.ws.readyState === WebSocket.OPEN) {
    current.ws.send(JSON.stringify({ type, value }))
  }
}

function startRtc(current) {
  const pc = new RTCPeerConnection()
  current.pc = pc
  const local = []
  const remote = []
  let offerSent = false
  pc.addTransceiver('video', { direction: 'recvonly' })
  pc.addTransceiver('audio', { direction: 'recvonly' })
  pc.ontrack = ({ streams, track }) => {
    if (session !== current || track.kind !== 'video' || !video.value) return
    video.value.srcObject = streams[0] || new MediaStream([track])
    video.value.play().catch(() => {})
  }
  pc.onconnectionstatechange = () => {
    if (pc.connectionState === 'closed') fail(current)
  }
  pc.oniceconnectionstatechange = () => {
    if (['failed', 'disconnected'].includes(pc.iceConnectionState)) {
      current.iceTimer ||= setTimeout(() => fail(current), 3000)
    } else if (['connected', 'completed'].includes(pc.iceConnectionState)) {
      clearTimeout(current.iceTimer)
      current.iceTimer = null
    }
  }
  pc.onicecandidate = ({ candidate }) => {
    if (!candidate) return
    if (offerSent) send(current, 'webrtc/candidate', candidate.candidate)
    else local.push(candidate.candidate)
  }
  const addIce = (candidate) => pc.addIceCandidate({ candidate, sdpMid: '0' }).catch(() => {})
  current.message = async ({ type, value }) => {
    if (typeof value !== 'string') return
    if (type === 'webrtc/answer') {
      await pc.setRemoteDescription({ type: 'answer', sdp: value })
      for (const candidate of remote.splice(0)) await addIce(candidate)
    } else if (type === 'webrtc/candidate' && value) {
      if (pc.remoteDescription) await addIce(value)
      else remote.push(value)
    }
  }
  pc.createOffer().then(async (offer) => {
    if (session !== current) return
    await pc.setLocalDescription(offer)
    send(current, 'webrtc/offer', pc.localDescription.sdp)
    offerSent = true
    for (const candidate of local.splice(0)) send(current, 'webrtc/candidate', candidate)
  }).catch(() => fail(current))
}

function startMse(current) {
  const source = new MediaSource()
  // go2rtc matches these exact codec identifiers in the MSE request.
  const codecs = ['avc1.640029', 'hvc1.1.6.L153.B0']
    .filter(codec => MediaSource.isTypeSupported(`video/mp4; codecs="${codec}"`))
  if (!codecs.length) { fail(current); return }
  let buffer
  let queuedBytes = 0
  const queue = []

  function flush() {
    if (session !== current || !buffer || buffer.updating) return
    try {
      if (buffer.buffered.length) {
        const end = buffer.buffered.end(buffer.buffered.length - 1)
        const start = buffer.buffered.start(0)
        if (video.value.currentTime < start || video.value.currentTime < end - 4) {
          video.value.currentTime = Math.max(start, end - 1)
        }
        if (start < end - 10) {
          buffer.remove(start, end - 8)
          return
        }
      }
      const next = queue.shift()
      if (next) {
        queuedBytes -= next.byteLength
        buffer.appendBuffer(next)
      }
    } catch { fail(current) }
  }

  source.addEventListener('sourceopen', () => send(current, 'mse', codecs.join(',')), { once: true })
  current.message = ({ type, value }) => {
    if (type !== 'mse' || buffer || typeof value !== 'string') return
    buffer = source.addSourceBuffer(value)
    buffer.addEventListener('updateend', flush)
    buffer.addEventListener('error', () => fail(current))
  }
  current.binary = (data) => {
    queuedBytes += data.byteLength
    if (queuedBytes > MAX_MSE_QUEUE_BYTES) { fail(current); return }
    queue.push(data)
    flush()
  }
  current.objectUrl = URL.createObjectURL(source)
  video.value.src = current.objectUrl
  video.value.play().catch(() => {})
}

function connect() {
  if (!mounted || !props.wsUrl || !video.value) return
  if (props.mode === 'mse' ? !window.MediaSource : !window.RTCPeerConnection) return
  const url = new URL(props.wsUrl, location.href)
  if (url.host !== location.host) return
  url.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
  const token = requestToken()
  const ws = new WebSocket(url.toString(), token ? ['labsentinel', 'bearer.' + token] : undefined)
  ws.binaryType = 'arraybuffer'
  const current = { ws, watchdog: null, ready: false }
  session = current
  current.watchdog = setTimeout(() => fail(current), FRAME_TIMEOUT_MS)
  ws.onopen = () => {
    try {
      if (props.mode === 'mse') startMse(current)
      else startRtc(current)
    } catch { fail(current) }
  }
  ws.onmessage = async ({ data }) => {
    if (session !== current) return
    if (data instanceof ArrayBuffer) {
      current.binary?.(data)
      return
    }
    try {
      const message = JSON.parse(data)
      if (message?.type === 'error') fail(current)
      else if (message) await current.message?.(message)
    } catch { fail(current) }
  }
  ws.onclose = () => fail(current)
}

function restart() {
  clearTimeout(retry)
  disconnect()
  attempts = 0
  connect()
}

watch(() => [props.wsUrl, props.mode], restart)
onMounted(() => { mounted = true; connect() })
onBeforeUnmount(() => { mounted = false; clearTimeout(retry); disconnect() })
</script>

<template>
  <video ref="video" muted autoplay playsinline @playing="observeFrames" @timeupdate="observeFrames" />
</template>

<style scoped>
video { display: block; width: 100%; height: 100%; object-fit: contain; }
</style>
