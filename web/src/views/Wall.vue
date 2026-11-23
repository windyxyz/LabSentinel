<!-- T-709：值班大屏 /wall（kiosk 模式）——多路实时画面网格 + 事件滚动。
     LS-051：全屏值守大屏（1/4/9 宫格、当日合规率、新告警浮窗、提示音/桌面通知开关）+ 检测框叠加。
     kiosk 启动：chrome --kiosk --app=http://127.0.0.1:8000/wall -->
<template>
  <div class="wall" :class="{ fullscreen: isFullscreen }">
    <header>
      <h2>LabSentinel 值班大屏</h2>
      <div class="hud">
        <span v-if="device" class="stat">{{ device.name }}</span>
        <span class="stat">今日警告及严重事件 <b>{{ violationsToday }}</b></span>
        <span class="stat">在场 <b>{{ inFrame }}</b> 人</span>
        <span class="clock">{{ now }}</span>
        <select v-if="groups.length" v-model="selectedGroup" aria-label="摄像头分组" class="ghost-select">
          <option value="">全部分组</option>
          <option v-for="group in groups" :key="group" :value="group">{{ group }}</option>
        </select>
        <select v-model.number="grid" aria-label="宫格切换" class="ghost-select">
          <option :value="1">1 宫格</option>
          <option :value="4">4 宫格</option>
          <option :value="9">9 宫格</option>
        </select>
        <button class="ghost" @click="toggleOverlay">{{ overlay ? '隐藏检测框' : '显示检测框' }}</button>
        <button class="ghost" @click="toggleMute">{{ muted ? '开启提示音' : '静音' }}</button>
        <button class="ghost" @click="toggleNotifications">{{ notifications ? '关闭桌面通知' : '开启桌面通知' }}</button>
        <button class="ghost" @click="toggleFullscreen">{{ isFullscreen ? '退出全屏' : '进入全屏值守大屏' }}</button>
        <button v-if="signedIn || device" class="ghost" @click="$emit('sign-out')">{{ device ? '退出设备' : '退出登录' }}</button>
      </div>
    </header>
    <div class="grid" :class="{ single: cols === 1 }" :style="{ '--wall-columns': cols }">
      <p v-if="!visibleCameras.length" class="muted wall-empty">当前范围内暂无摄像头。</p>
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
        <li v-for="e in visibleEvents" :key="e.id" :class="'L' + e.level">
          <span class="ts">{{ (e.ts || '').slice(11, 19) }}</span>
          <span class="ty">{{ TYPE_TEXT[e.type] || e.type }}</span>
          <span class="de">{{ e.detail }}</span>
          <div v-if="device?.can_triage && (!e.status || e.status === 'new')" class="event-actions">
            <button type="button" class="ghost" :disabled="triaging.has(e.id)" @click="triage(e, 'acked')">确认</button>
            <button type="button" class="ghost" :disabled="triaging.has(e.id)" @click="triage(e, 'false_positive')">误报</button>
          </div>
          <small v-else-if="device?.can_triage" class="muted event-state">{{ e.status === 'false_positive' ? '已标误报' : '已处置' }}</small>
        </li>
      </ul>
      <p v-if="triageError" role="alert" class="st-err">{{ triageError }}</p>
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
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { api, connectEvents, TYPE_TEXT } from '../api'
import CameraPreview from '../components/CameraPreview.vue'
import VideoOverlayCanvas from '../components/VideoOverlayCanvas.vue'

const cameras = ref([])
const props = defineProps({ readonly: Boolean, signedIn: Boolean, device: Object })
defineEmits(['sign-out'])
const tracks = ref({})
const events = ref([])
const now = ref('')
const preferenceKey = `ls-wall-view-${props.device?.id ?? (props.readonly ? 'share' : 'console')}`
let preferences = {}
try { preferences = JSON.parse(localStorage.getItem(preferenceKey) || '{}') || {} } catch { /* 使用默认布局 */ }
const grid = ref([1, 4, 9].includes(preferences.grid) ? preferences.grid : 9)
const selectedGroup = ref(typeof preferences.group === 'string' ? preferences.group : '')
watch([grid, selectedGroup], () => {
  localStorage.setItem(preferenceKey, JSON.stringify({ grid: grid.value, group: selectedGroup.value }))
})
const overlay = ref(true)    // 检测框显隐
const muted = ref(true)      // 默认静音（浏览器自动播放策略；用户点击解锁）
const notifications = ref(false)
const isFullscreen = ref(false)
const toast = ref(null)
const violationsToday = ref('—')
const triaging = ref(new Set())
const triageError = ref('')

let clockTimer = null
let camTimer = null
let stopWs = null
let audioCtx = null
let toastTimer = null
let disposed = false

const cols = computed(() => (grid.value >= 9 ? 3 : grid.value >= 4 ? 2 : 1))
const groups = computed(() => [...new Set(cameras.value.map(camera => camera.group_name).filter(Boolean))].sort())
const groupCameras = computed(() => !selectedGroup.value ? cameras.value : cameras.value.filter(camera => camera.group_name === selectedGroup.value))
const visibleCameras = computed(() => groupCameras.value.slice(0, grid.value))
const visibleEvents = computed(() => {
  if (!selectedGroup.value) return events.value
  const cameraIds = new Set(groupCameras.value.map(camera => camera.id))
  return events.value.filter(event => cameraIds.has(event.camera_id))
})
const inFrame = computed(() => visibleCameras.value.reduce((n, c) => n + (tracks.value[c.id] || 0), 0))

async function loadCameras() {
  try {
    const cs = await api.cameras()
    cameras.value = Array.isArray(cs) ? cs.filter(camera => camera && (!props.device || camera.type !== 'folder')) : []
    if (selectedGroup.value && !groups.value.includes(selectedGroup.value)) selectedGroup.value = ''
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
  events.value = [ev, ...events.value.filter(event => event.id !== ev.id)].slice(0, 50)
  if (selectedGroup.value && !groupCameras.value.some(camera => camera.id === ev.camera_id)) return
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

async function triage(event, status) {
  if (triaging.value.has(event.id)) return
  triaging.value.add(event.id)
  triageError.value = ''
  try {
    await api.setEventStatus(event.id, status)
    const current = events.value.find(item => item.id === event.id)
    if (current) current.status = status
  } catch (err) { triageError.value = String(err.message || err) }
  finally { triaging.value.delete(event.id) }
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
.wall { display: grid; grid-template-columns: minmax(0, 1fr) 340px; grid-template-rows: auto minmax(0, 1fr);
        height: 100vh; background: var(--bg-base); color: var(--text-primary); }
.wall.fullscreen { grid-template-columns: 1fr; }
.wall.fullscreen aside { display: none; }
header { grid-column: 1 / -1; display: flex; align-items: center; justify-content: space-between;
         padding: 10px 18px; border-bottom: 1px solid var(--divider); gap: 12px; flex-wrap: wrap; }
h2 { margin: 0; font-size: 20px; }
.hud { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
.stat { font-size: 13px; color: var(--text-secondary); }
.stat b { color: var(--accent-strong); font-variant-numeric: tabular-nums; }
.clock { font-variant-numeric: tabular-nums; color: var(--text-secondary); }
.ghost-select { margin: 0; }
.grid { display: grid; grid-template-columns: repeat(var(--wall-columns), minmax(0, 1fr)); gap: 8px; padding: 8px; overflow: auto; }
.cell { position: relative; background: var(--card-bg); border: 1px solid var(--stroke-secondary); border-radius: 6px;
        min-height: 200px; min-width: 0; display: flex; align-items: center; justify-content: center; }
.wall-empty { grid-column: 1 / -1; padding: 12px; }
.camera-preview { position: absolute; inset: 0; }
.capture { position: absolute; bottom: 6px; right: 6px; z-index: 3; }
.meta { position: absolute; left: 8px; top: 6px; font-size: 12px; color: var(--text-primary); background: var(--bg-base); border-radius: 4px; padding: 2px 6px; z-index: 3; }
aside { border-left: 1px solid var(--divider); padding: 10px; overflow: auto; }
aside h3 { margin: 4px 0 10px; font-size: 15px; }
aside ul { list-style: none; margin: 0; padding: 0; }
aside li { padding: 6px 8px; margin-bottom: 6px; background: var(--card-bg); border-left: 3px solid var(--stroke-secondary);
           border-radius: 4px; font-size: 13px; }
aside li.L3 { border-left-color: var(--l3); }
aside li.L2 { border-left-color: var(--l2); }
.ts { color: var(--text-secondary); margin-right: 6px; font-variant-numeric: tabular-nums; }
.ty { color: var(--accent-strong); margin-right: 6px; }
.de { color: var(--text-primary); overflow-wrap: anywhere; }
.event-actions { display: flex; gap: 6px; margin-top: 5px; }
.event-state { display: block; }
.alert-toast { position: fixed; right: 16px; bottom: 16px; max-width: min(380px, calc(100vw - 32px)); z-index: 50;
  background: var(--dialog-bg); border: 1px solid var(--stroke-secondary); border-left: 4px solid var(--l2);
  border-radius: 8px; padding: 10px 12px; display: flex; flex-direction: column; gap: 4px;
  box-shadow: 0 8px 24px rgba(0,0,0,.45); cursor: pointer; }
.alert-toast.L3 { border-left-color: var(--l3); }
.alert-toast b { color: var(--l2); font-size: 13px; }
.alert-toast span { color: var(--text-primary); font-size: 12.5px; }
.toast-enter-active, .toast-leave-active { transition: opacity .2s ease, transform .2s ease; }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translateY(8px); }
@media (max-width: 850px) {
  .wall { grid-template-columns: minmax(0, 1fr); grid-template-rows: auto auto auto; height: auto; min-height: 100vh; }
  .grid:not(.single) { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .cell { min-height: 120px; aspect-ratio: 16 / 9; }
  aside { border-left: 0; border-top: 1px solid var(--divider); }
}
</style>
