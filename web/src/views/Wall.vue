<!-- T-709：值班大屏 /wall（kiosk 模式）——多路实时画面网格 + 事件滚动。
     kiosk 启动：chrome --kiosk --app=http://127.0.0.1:8000/wall -->
<template>
  <div class="wall">
    <header>
      <h2>LabSentinel 值班大屏</h2>
      <span class="clock">{{ now }}</span>
    </header>
    <div class="grid" :style="{ gridTemplateColumns: `repeat(${cols}, 1fr)` }">
      <div v-for="c in cameras.slice(0, 9)" :key="c.id" class="cell">
        <WebrtcPlayer v-if="isVideo(c)" :ws-url="`/ws/webrtc/${c.id}`" :label="c.name" />
        <div v-else class="placeholder">{{ c.name }}（{{ c.type }} 源无实时流）</div>
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
  </div>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { api, connectEvents } from '../api'
import WebrtcPlayer from '../components/WebrtcPlayer.vue'

const cameras = ref([])
const events = ref([])
const now = ref('')
let clockTimer = null
let stopWs = null

const cols = computed(() => (cameras.value.length <= 1 ? 1 : cameras.value.length <= 4 ? 2 : 3))

function isVideo(c) {
  return c && (c.type === 'rtsp' || c.type === 'file' || c.type === 'usb')
}

async function loadCameras() {
  try {
    cameras.value = await api.cameras()
  } catch { /* 忽略：下一轮 */ }
}

onMounted(async () => {
  await loadCameras()
  try {
    const list = await api.events({ limit: 50 })
    events.value = (list || []).slice(0, 50)
  } catch { /* ignore */ }
  stopWs = connectEvents((ev) => {
    events.value.unshift(ev)
    events.value = events.value.slice(0, 50)
  }, () => {})
  clockTimer = setInterval(() => {
    now.value = new Date().toLocaleString('zh-CN', { hour12: false })
  }, 1000)
  setInterval(loadCameras, 30000)  // 摄像头增删自动反映
})

onBeforeUnmount(() => {
  if (clockTimer) clearInterval(clockTimer)
  if (stopWs) stopWs()
})
</script>

<style scoped>
.wall { display: grid; grid-template-columns: 1fr 340px; grid-template-rows: auto 1fr;
        height: 100vh; background: #0b0f14; color: #dfe7ef; }
header { grid-column: 1 / -1; display: flex; align-items: center; justify-content: space-between;
         padding: 10px 18px; border-bottom: 1px solid #1c2733; }
h2 { margin: 0; font-size: 20px; }
.clock { font-variant-numeric: tabular-nums; color: #8fa3b8; }
.grid { display: grid; gap: 8px; padding: 8px; overflow: auto; }
.cell { position: relative; background: #101821; border: 1px solid #1c2733; border-radius: 6px;
        min-height: 200px; display: flex; align-items: center; justify-content: center; }
.placeholder { color: #5a6f84; font-size: 13px; }
.meta { position: absolute; left: 8px; bottom: 6px; font-size: 12px; color: #8fa3b8; }
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
</style>
