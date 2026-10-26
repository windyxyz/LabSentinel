<!--
  WebrtcPlayer · go2rtc WebRTC 实时画面播放器（零第三方依赖，手写信令）
  ====================================================================

  用法（任意视图中，弹层内给定固定宽高的容器即可）：

    <WebrtcPlayer
      v-if="playerUrl"
      :ws-url="playerUrl"
      label="1 号摄像头"
    />

  wsUrl 为 go2rtc 信令 WebSocket 的【完整地址】，例如：
    ws://localhost:1984/api/ws?src=1      （src 为摄像头 ID，即 go2rtc 里的流名）
  部署时把 host 换成 go2rtc 所在机器；走反向代理时用 wss://。
  本地无 go2rtc 时可用 mock 验证：node web/mock/go2rtc-mock.mjs

  信令协议（go2rtc 官方 demo 同款）：
    1. new RTCPeerConnection() → addTransceiver('video'/'audio', {direction:'recvonly'})
       → createOffer → setLocalDescription(offer)
    2. ws 发送  { type: 'webrtc', value: btoa(pc.localDescription.sdp) }
    3. 收到 { type: 'webrtc', value }        → setRemoteDescription({type:'answer', sdp: atob(value)})
    4. pc.onicecandidate → ws 发送 { type: 'webrtc/candidate', value: btoa(candidate.candidate) }
       （candidate 为 null 表示收集完成，value 发空字符串，照发）
    5. 收到 { type: 'webrtc/candidate', value } → pc.addIceCandidate({ candidate: atob(value) })
    6. pc.ontrack → video.srcObject = event.streams[0] 并 play()

  断流重连：ws 关闭、等 answer 超时、或 ICE failed/disconnected 持续 3s 后重连；
  指数退避 0.5s 起步 ×2、上限 15s；ontrack 真正拿到流后计数清零。
  组件卸载时彻底清理：关 ws、关 pc、取消全部定时器。
-->
<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

const props = defineProps({
  /** go2rtc 信令 WebSocket 完整地址，如 ws://localhost:1984/api/ws?src=1 */
  wsUrl: { type: String, default: '' },
  /** 左上角叠加显示的流名称（可选） */
  label: { type: String, default: '' },
})

/* ---------- 状态角标 ---------- */
// connecting: 连接中 / connected: 已连接 / reconnecting: 重连中 / disconnected: 已断开
const status = ref('disconnected')
const retryCount = ref(0)
const STATUS_TEXT = {
  connecting: '连接中',
  connected: '已连接',
  reconnecting: '重连中',
  disconnected: '已断开',
}
const statusText = computed(() => {
  const base = STATUS_TEXT[status.value] ?? status.value
  return status.value === 'reconnecting' ? `${base}(第${retryCount.value}次)` : base
})

/* ---------- 重连参数（与 web/mock/go2rtc-mock.mjs 自测客户端保持一致） ---------- */
const BACKOFF_BASE = 500      // 首次重连延迟 ms
const BACKOFF_MAX = 15000     // 退避上限 ms
const ICE_RECOVER_MS = 3000   // ICE failed/disconnected 的观察窗口，超过仍未恢复才重连
const OFFER_TIMEOUT_MS = 10000 // 发出 offer 后等 answer 的超时

/* ---------- 会话内部变量（非响应式） ---------- */
let ws = null
let pc = null
let retryTimer = null
let iceTimer = null
let offerTimer = null
let attempts = 0     // 连续失败次数（驱动指数退避）
let stopped = false  // 组件卸载后不再发起任何连接

const videoRef = ref(null)

function clearTimers() {
  if (retryTimer) { clearTimeout(retryTimer); retryTimer = null }
  if (iceTimer) { clearTimeout(iceTimer); iceTimer = null }
  if (offerTimer) { clearTimeout(offerTimer); offerTimer = null }
}

/** 销毁当前信令 + PeerConnection 会话（幂等） */
function destroySession() {
  if (iceTimer) { clearTimeout(iceTimer); iceTimer = null }
  if (offerTimer) { clearTimeout(offerTimer); offerTimer = null }
  if (ws) {
    ws.onopen = ws.onmessage = ws.onclose = ws.onerror = null
    try { ws.close() } catch { /* 已关闭则忽略 */ }
    ws = null
  }
  if (pc) {
    pc.ontrack = pc.onicecandidate = pc.oniceconnectionstatechange = pc.onconnectionstatechange = null
    try { pc.close() } catch { /* 已关闭则忽略 */ }
    pc = null
  }
}

/** 安排一次指数退避重连 */
function scheduleReconnect() {
  destroySession()
  if (stopped) return
  attempts += 1
  retryCount.value = attempts
  status.value = 'reconnecting'
  const delay = Math.min(BACKOFF_BASE * 2 ** (attempts - 1), BACKOFF_MAX)
  retryTimer = setTimeout(connect, delay)
}

/** 标记"流可能已断"，观察窗口过后仍未恢复则重连 */
function markStreamBroken() {
  if (stopped || !pc) return
  if (!iceTimer) iceTimer = setTimeout(() => scheduleReconnect(), ICE_RECOVER_MS)
}

/** 发起一次全新连接（每次都新建 RTCPeerConnection，避免旧实例状态污染） */
function connect() {
  if (stopped) return
  if (!props.wsUrl) { status.value = 'disconnected'; return }
  status.value = attempts > 0 ? 'reconnecting' : 'connecting'

  pc = new RTCPeerConnection()
  // 只收不发：先按 go2rtc 约定加 video/audio 两个收流收发器，再 createOffer
  pc.addTransceiver('video', { direction: 'recvonly' })
  pc.addTransceiver('audio', { direction: 'recvonly' })

  pc.ontrack = (ev) => {
    if (!videoRef.value) return
    videoRef.value.srcObject = ev.streams?.[0] || new MediaStream([ev.track])
    videoRef.value.play().catch(() => {}) // autoplay 被浏览器策略拦截时忽略
    attempts = 0            // 真正拿到流才算成功，退避计数清零
    retryCount.value = 0
    status.value = 'connected'
    if (iceTimer) { clearTimeout(iceTimer); iceTimer = null }
    if (offerTimer) { clearTimeout(offerTimer); offerTimer = null }
  }

  // 本端 ICE 候选 → 信令服务器；candidate 为 null 表示收集完成，value 发空串照发
  pc.onicecandidate = (ev) => {
    if (!ws || ws.readyState !== WebSocket.OPEN) return
    const value = ev.candidate ? btoa(ev.candidate.candidate) : ''
    ws.send(JSON.stringify({ type: 'webrtc/candidate', value }))
  }

  // ICE 层断链：failed/disconnected 持续 ICE_RECOVER_MS 未恢复 → 重连
  pc.oniceconnectionstatechange = () => {
    if (!pc || stopped) return
    const st = pc.iceConnectionState
    if (st === 'failed' || st === 'disconnected') markStreamBroken()
    else if (st === 'connected' || st === 'completed') {
      if (iceTimer) { clearTimeout(iceTimer); iceTimer = null }
    }
  }

  // 连接层兜底（部分浏览器上比 ICE 状态更早暴露失败）
  pc.onconnectionstatechange = () => {
    if (pc && (pc.connectionState === 'failed' || pc.connectionState === 'disconnected')) markStreamBroken()
  }

  ws = new WebSocket(props.wsUrl)

  ws.onopen = async () => {
    try {
      const offer = await pc.createOffer()
      await pc.setLocalDescription(offer)
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: 'webrtc', value: btoa(pc.localDescription.sdp) }))
        // 等 answer 超时兜底：服务端半死不活时不至于永远卡在"连接中"
        offerTimer = setTimeout(() => scheduleReconnect(), OFFER_TIMEOUT_MS)
      }
    } catch (e) {
      console.error('[WebrtcPlayer] 创建/发送 offer 失败', e)
    }
  }

  ws.onmessage = (ev) => {
    if (!pc) return
    let msg
    try { msg = JSON.parse(ev.data) } catch { return }
    if (msg.type === 'webrtc' && msg.value) {
      if (offerTimer) { clearTimeout(offerTimer); offerTimer = null }
      // answer sdp（base64 编码）→ 设为远端描述
      pc.setRemoteDescription({ type: 'answer', sdp: atob(msg.value) }).catch((e) => {
        console.error('[WebrtcPlayer] setRemoteDescription 失败', e)
        scheduleReconnect() // 非法 answer 视为握手失败，走退避重连
      })
    } else if (msg.type === 'webrtc/candidate' && msg.value) {
      pc.addIceCandidate({ candidate: atob(msg.value) }).catch(() => {}) // 候选不可达属正常，静默
    }
  }

  // 信令断开（网络/服务端掉线/对端主动关闭）→ 退避重连
  ws.onclose = () => { if (!stopped) scheduleReconnect() }
  ws.onerror = () => {} // ws 出错后必然触发 onclose，统一在那里处理
}

/** 按新地址整体重启（切流/换源时调用） */
function restart() {
  clearTimers()
  destroySession()
  attempts = 0
  retryCount.value = 0
  if (props.wsUrl) connect()
  else status.value = 'disconnected'
}

watch(() => props.wsUrl, restart)
onMounted(() => { if (props.wsUrl) connect() })

onBeforeUnmount(() => {
  stopped = true
  clearTimers()
  destroySession()
})
</script>

<template>
  <div class="webrtc-player">
    <video ref="videoRef" muted autoplay playsinline></video>
    <span v-if="label" class="tag label">{{ label }}</span>
    <span class="tag status" :class="'st-' + status">{{ statusText }}</span>
  </div>
</template>

<style scoped>
.webrtc-player {
  position: relative;
  width: 100%;
  height: 100%;
  min-height: 240px;
  background: #000;
  border-radius: 8px;
  overflow: hidden;
}
.webrtc-player video {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: contain;
  background: #000;
}
.webrtc-player .tag {
  position: absolute;
  padding: 1px 8px;
  border-radius: 10px;
  font-size: 12px;
  line-height: 18px;
  background: rgba(0, 0, 0, .55);
  border: 1px solid var(--stroke-default);
  pointer-events: none;
}
.webrtc-player .tag.label { top: 8px; left: 8px; color: var(--text); }
.webrtc-player .tag.status { bottom: 8px; left: 8px; }
.st-connecting { color: var(--accent); }
.st-connected { color: #53d769; } /* 绿色：现有调色板无绿，播放器内单独使用 */
.st-reconnecting { color: var(--l2); }
.st-disconnected { color: var(--l3); }
</style>
