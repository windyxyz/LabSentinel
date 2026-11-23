<!-- T-709：值班大屏 /wall（kiosk 模式）——多路实时画面网格 + 事件滚动。
     LS-051：全屏值守大屏（1/4/9 宫格、当日合规率、新告警浮窗、提示音/桌面通知开关）+ 检测框叠加。
     kiosk 启动：chrome --kiosk --app=http://127.0.0.1:8000/wall -->
<template>
  <div class="wall" :class="{ fullscreen: isFullscreen }">
    <header>
      <h2>LabSentinel 值班大屏</h2>
      <div class="hud">
        <span class="stat">今日警告及严重事件 <b>{{ violationsToday }}</b></span>
        <span class="stat">在场 <b>{{ inFrame }}</b> 人</span>
        <span class="clock">{{ now }}</span>
        <select v-model.number="grid" aria-label="宫格切换" class="ghost-select">
          <option :value="1">1 宫格</option>
          <option :value="4">4 宫格</option>
          <option :value="9">9 宫格</option>
        </select>
        <button class="ghost" @click="toggleOverlay">{{ overlay ? '隐藏检测框' : '显示检测框' }}</button>
        <button class="ghost" @click="toggleMute">{{ muted ? '开启提示音' : '静音' }}</button>
        <button class="ghost" @click="toggleNotifications">{{ notifications ? '关闭桌面通知' : '开启桌面通知' }}</button>
        <button class="ghost" @click="toggleFullscreen">{{ isFullscreen ? '退出全屏' : '进入全屏值守大屏' }}</button>
        <button v-if="signedIn" class="ghost" @click="$emit('sign-out')">退出登录</button>
      </div>
    </header>
    <div class="grid" :style="{ gridTemplateColumns: `repeat(${cols}, 1fr)` }">
      <div v-for="c in visibleCameras" :key="c.id" class="cell">
        <CameraPreview class="camera-preview" :camera-id="c.id" :camera-type="c.type" :name="c.name" />
        <VideoOverlayCanvas v-if="c.type !== 'folder'" :camera-id="c.id" :visible="overlay"
          @tracks="n => tracks[c.id] = n" />
        <button v-if="!readonly && c.type !== 'folder'" class="ghost capture" @click="capture(c.id)">抓拍入池</button>
        <div class="meta">{{ c.name }} · {{ c.status }}</div>
      </div>
    </div>
    <aside>
      <h3>实时事件</h3>
      <ul>
        <li v-for="e in events" :key="e.id" :class="'L' + e.level">
          <span class="ts">{{ (e.ts || '').slice(11, 19) }}</span>
          <span class="ty">{{ e.type }}</span>
          <span class="de">{{ e.detail }}</span>
        </li>
      </ul>
    </aside>
    <!-- LS-051：新告警右下角浮窗 -->
    <transition name="toast">
      <div v-if="toast" class="alert-toast" :class="'L' + toast.level" @click="toast = null">
        <b>L{{ toast.level }} · {{ toast.type }}</b>
        <span>{{ toast.detail }}</span>
      </div>
    </transition>
  </div>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { api, connectEvents, TYPE_TEXT } from '../api'
import CameraPreview from '../components/CameraPreview.vue'
import VideoOverlayCanvas from '../components/VideoOverlayCanvas.vue'

const cameras = ref([])
defineProps({ readonly: Boolean, signedIn: Boolean })
defineEmits(['sign-out'])
const tracks = ref({})
const events = ref([])
const now = ref('')
const grid = ref(9)          // 1/4/9 宫格
const overlay = ref(true)    // 检测框显隐
const muted = ref(true)      // 默认静音（浏览器自动播放策略；用户点击解锁）
const notifications = ref(false)
const isFullscreen = ref(false)
const toast = ref(null)
const violationsToday = ref('—')

let clockTimer = null
let camTimer = null
let stopWs = null
let audioCtx = null
let toastTimer = null
let disposed = false

const cols = computed(() => (grid.value >= 9 ? 3 : grid.value >= 4 ? 2 : 1))
const visibleCameras = computed(() => cameras.value.slice(0, grid.value))
const inFrame = computed(() => visibleCameras.value.reduce((n, c) => n + (tracks.value[c.id] || 0), 0))

async function loadCameras() {
  try {
    const cs = await api.cameras()
    cameras.value = Array.isArray(cs) ? cs : []
  } catch { /* 忽略：下一轮 */ }
}

async function capture(id) {
  try {
    const result = await api.datasetCapture(id)
    toast.value = { level: 1, type: '标注池', detail: `已加入样本 #${result.id}` }
  } catch (err) { toast.value = { level: 1, type: '抓拍失败', detail: err.message } }
  clearTimeout(toastTimer)
  toastTimer = setTimeout(() => (toast.value = null), 6000)
}

async function loadStats() {
  try {
    const s = await api.statsDaily({ days: 1 })
    const day = Array.isArray(s?.days) ? s.days[0] : null
    if (day) violationsToday.value = (Number(day.l2) || 0) + (Number(day.l3) || 0)
  } catch { /* 旧服务端无字段：保持 — */ }
}

function beep() {
  if (muted.value) return
  try {
    audioCtx = audioCtx || new (window.AudioContext || window.webkitAudioContext)()
    const osc = audioCtx.createOscillator()
    const gain = audioCtx.createGain()
    osc.connect(gain)
    gain.connect(audioCtx.destination)
    osc.frequency.value = 880
    gain.gain.value = 0.08
    osc.start()
    osc.stop(audioCtx.currentTime + 0.18)
  } catch { /* 浏览器策略限制：忽略 */ }
}

function onNewEvent(ev) {
  events.value.unshift(ev)
  events.value = events.value.slice(0, 50)
  if (Number(ev.level) >= 2) {
    toast.value = ev
    clearTimeout(toastTimer)
    toastTimer = setTimeout(() => (toast.value = null), 6000)
    beep()
    // 页面隐藏时桌面通知（若已授权）
    if (notifications.value && typeof Notification !== 'undefined' && Notification.permission === 'granted' && document.hidden) {
      try {
        new Notification(`LabSentinel L${ev.level}`, {
          body: `${TYPE_TEXT[ev.type] || ev.type}：${ev.detail || ''}`,
          tag: 'ls-' + ev.id,
        })
      } catch { /* ignore */ }
    }
  }
}

function toggleOverlay() { overlay.value = !overlay.value }
function toggleMute() {
  muted.value = !muted.value
  if (!muted.value) beep()
}
async function toggleNotifications() {
  if (notifications.value) { notifications.value = false; return }
  if (typeof Notification !== 'undefined') {
    notifications.value = await Notification.requestPermission() === 'granted'
  }
}
async function toggleFullscreen() {
  try {
    if (!document.fullscreenElement) {
      await document.documentElement.requestFullscreen()
      isFullscreen.value = true
    } else {
      await document.exitFullscreen()
      isFullscreen.value = false
    }
  } catch { /* 浏览器拒绝：忽略 */ }
}
function onFsChange() { isFullscreen.value = !!document.fullscreenElement }

onMounted(async () => {
  await loadCameras()
  await loadStats()
  try {
    const list = await api.events({ limit: 50 })
    events.value = (Array.isArray(list?.items) ? list.items : Array.isArray(list) ? list : []).slice(0, 50)
  } catch { /* ignore */ }
  if (disposed) return
  stopWs = connectEvents(onNewEvent, () => {})
  clockTimer = setInterval(() => {
    now.value = new Date().toLocaleString('zh-CN', { hour12: false })
  }, 1000)
  camTimer = setInterval(() => { loadCameras(); loadStats() }, 30000)
  document.addEventListener('fullscreenchange', onFsChange)
})

onBeforeUnmount(() => {
  disposed = true
  if (audioCtx) audioCtx.close()
  if (clockTimer) clearInterval(clockTimer)
  if (camTimer) clearInterval(camTimer)
  if (toastTimer) clearTimeout(toastTimer)
  if (stopWs) stopWs()
  document.removeEventListener('fullscreenchange', onFsChange)
})
</script>

<style scoped>
.wall { display: grid; grid-template-columns: 1fr 340px; grid-template-rows: auto 1fr;
        height: 100vh; background: var(--bg-base); color: var(--text-primary); }
.wall.fullscreen { grid-template-columns: 1fr; }
.wall.fullscreen aside { display: none; }
header { grid-column: 1 / -1; display: flex; align-items: center; justify-content: space-between;
         padding: 10px 18px; border-bottom: 1px solid #1c2733; gap: 12px; flex-wrap: wrap; }
h2 { margin: 0; font-size: 20px; }
.hud { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
.stat { font-size: 13px; color: #8fa3b8; }
.stat b { color: #67b7ff; font-variant-numeric: tabular-nums; }
.clock { font-variant-numeric: tabular-nums; color: #8fa3b8; }
.ghost-select { background: #101821; color: #dfe7ef; border: 1px solid #1c2733; border-radius: 6px; padding: 4px 8px; }
.grid { display: grid; gap: 8px; padding: 8px; overflow: auto; }
.cell { position: relative; background: #101821; border: 1px solid #1c2733; border-radius: 6px;
        min-height: 200px; display: flex; align-items: center; justify-content: center; }
.camera-preview { position: absolute; inset: 0; }
.capture { position: absolute; bottom: 6px; right: 6px; z-index: 3; }
.meta { position: absolute; left: 8px; top: 6px; font-size: 12px; color: var(--text-primary); background: var(--bg-base); border-radius: 4px; padding: 2px 6px; z-index: 3; }
aside { border-left: 1px solid #1c2733; padding: 10px; overflow: auto; }
aside h3 { margin: 4px 0 10px; font-size: 15px; }
aside ul { list-style: none; margin: 0; padding: 0; }
aside li { padding: 6px 8px; margin-bottom: 6px; background: #101821; border-left: 3px solid #3a4a5c;
           border-radius: 4px; font-size: 13px; }
aside li.L3 { border-left-color: #e5484d; }
aside li.L2 { border-left-color: #f5a524; }
.ts { color: #5a6f84; margin-right: 6px; font-variant-numeric: tabular-nums; }
.ty { color: #67b7ff; margin-right: 6px; }
.de { color: #b9c7d6; }
.alert-toast { position: fixed; right: 16px; bottom: 16px; max-width: 380px; z-index: 50;
  background: #101821; border: 1px solid #1c2733; border-left: 4px solid #f5a524;
  border-radius: 8px; padding: 10px 12px; display: flex; flex-direction: column; gap: 4px;
  box-shadow: 0 8px 24px rgba(0,0,0,.45); cursor: pointer; }
.alert-toast.L3 { border-left-color: #e5484d; }
.alert-toast b { color: #f2c14e; font-size: 13px; }
.alert-toast span { color: #b9c7d6; font-size: 12.5px; }
.toast-enter-active, .toast-leave-active { transition: opacity .2s ease, transform .2s ease; }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translateY(8px); }
</style>
