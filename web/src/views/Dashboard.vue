<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api, connectEvents, TYPE_TEXT } from '../api'
import { notifyEvent, ensurePermission, permissionState, notifyEnabled, setNotifyEnabled } from '../notify'
import AsyncState from '../components/AsyncState.vue'

const wsState = ref('连接中')
// 任务115：桌面通知状态与开关
const notifPerm = ref(permissionState())
const notifOn = ref(notifyEnabled())

async function enableNotify() {
  notifPerm.value = await ensurePermission()
  if (notifPerm.value === 'granted') {
    setNotifyEnabled(true)
    notifOn.value = true
  }
}
function toggleNotify() {
  setNotifyEnabled(!notifOn.value)
  notifOn.value = !notifOn.value
}

// 任务117：版本更新检查（未配置/失败静默，不打扰）
const upd = ref(null)
const updIgnored = ref(localStorage.getItem('LS_UPDATE_IGNORE') || '')

async function checkUpdate() {
  try {
    upd.value = await api.updateCheck({ refresh: 1 })
  } catch { upd.value = { error: '检查失败' } }
}
function ignoreUpdate() {
  if (upd.value?.latest) {
    localStorage.setItem('LS_UPDATE_IGNORE', upd.value.latest)
    updIgnored.value = upd.value.latest
  }
}
onMounted(() => { checkUpdate() })
const sys = ref(null)
// R2-303：GPU 加速状态（依据 /api/system.providers，onnxruntime 构建内建 EP）
const gpuOn = computed(() => (sys.value?.providers || []).includes('DmlExecutionProvider'))
const gpuHint = computed(() => {
  const providers = sys.value?.providers || []
  const gpu = providers.filter((p) => !/CPU|Azure/i.test(p))
  return gpuOn.value
    ? `GPU 加速：开启（${gpu.join('/')}）`
    : 'GPU 加速：未启用（CPU 推理；可选 GPU(DML) 变体包随 0.3.0 提供）'
})
const feed = ref([])
const loading = ref(true)
const error = ref('')
const notice = ref('')
let pollTimer = null
const maint = ref(null)  // 任务68：磁盘水位

async function reload() {
  error.value = ''
  try {
    const s = await api.system()
    sys.value = s          // 轮询失败时保留旧值（notice 提示），成功才覆盖
    notice.value = ''
  } catch (e) {
    if (sys.value) notice.value = '刷新失败，正在重试…（仍显示上一次数据）'
    else error.value = String(e.message || e)
  } finally {
    loading.value = false
  }
  try {
    maint.value = await api.maintenanceStatus()  // 任务68：磁盘水位（失败静默保留旧值）
  } catch { /* 静默 */ }
}

onMounted(async () => {
  // 2026-10-30 审阅修复：onUnmounted 必须在首个 await 之前注册（Vue 只在同步段收集生命周期钩子，
  // 原写在 await reload() 之后是 no-op → WS 永不清理）；WS 建立同样前移，不依赖首屏数据
  const close = connectEvents(
    (ev) => {
      feed.value = [ev, ...feed.value].slice(0, 100)
      notifyEvent(ev)  // 任务115：页面隐藏且有权限时弹桌面通知（L2/L3，tag 去重）
    },
    (s) => (wsState.value = s),
  )
  onUnmounted(close)
  pollTimer = setInterval(reload, 5000)  // 任务66：卡片 5s 轮询；失败保留旧值只出提示
  await reload()
})
onUnmounted(() => clearInterval(pollTimer))
</script>

<template>
  <AsyncState :loading="loading" :error="error" :notice="notice" @retry="reload">
    <div class="cards" v-if="sys">
      <div class="card" :title="`onnxruntime providers: ${(sys.providers || []).join(', ') || '探测失败'}`">
        <div class="muted">后端</div>
        <div class="num">{{ sys.backend }}</div>
        <div class="muted" style="font-size:12px; margin-top:4px">{{ gpuHint }}</div>
      </div>
      <div class="card"><div class="muted">今日事件</div><div class="num">{{ Object.values(sys.today_counts || {}).reduce((a, b) => a + b, 0) }}</div></div>
      <div class="card"><div class="muted">未处置</div><div class="num" :style="{ color: sys.unhandled_today > 0 ? 'var(--l2)' : undefined }">{{ sys.unhandled_today ?? 0 }}</div></div>
      <div class="card"><div class="muted">警告(L2)</div><div class="num">{{ sys.today_counts?.['2'] || 0 }}</div></div>
      <div class="card"><div class="muted">严重(L3)</div><div class="num">{{ sys.today_counts?.['3'] || 0 }}</div></div>
      <div class="card"><div class="muted">实时通道</div><div class="num" :style="{ fontSize: '18px', paddingTop: '6px' }">{{ wsState }}</div></div>
      <div v-if="upd" class="card" :title="upd.error || '只提示新版本，不自动下载安装'">
        <div class="muted">版本</div>
        <div style="padding-top:6px; font-size:14px">
          {{ upd.current || '-' }}
          <button class="ghost" style="font-size:12px" @click="checkUpdate">检查更新</button>
          <div v-if="upd.update_available && upd.latest !== updIgnored" class="muted"
               style="font-size:12px; margin-top:4px">
            发现新版本 {{ upd.latest }}<span v-if="upd.channel">（{{ upd.channel }}）</span>
            <a v-if="upd.url" :href="upd.url" target="_blank" rel="noopener">查看</a> ·
            <a href="#" @click.prevent="ignoreUpdate">忽略此版本</a>
          </div>
          <div v-else-if="upd.error" class="muted" style="font-size:12px">检查失败（不打扰）</div>
        </div>
      </div>
      <div class="card" :title="`通知权限：${notifPerm}；页面在后台时 L2/L3 弹桌面通知`">
        <div class="muted">桌面通知</div>
        <div style="padding-top:6px">
          <button v-if="notifPerm !== 'granted'" class="ghost" @click="enableNotify">开启桌面通知</button>
          <label v-else class="muted" style="font-size:13px">
            <input type="checkbox" :checked="notifOn" @change="toggleNotify" style="margin:0 4px 0 0" />
            {{ notifOn ? '已开启' : '已暂停' }}</label>
        </div>
      </div>
      <div v-if="maint" class="card" :style="maint.low_disk ? 'border-color:#dc2626; background:rgba(220,38,38,.08)' : undefined"
           :title="maint.forecast_days != null ? `按当前增速约 ${maint.forecast_days} 天后磁盘吃紧（估算值）` : '数据目录所在磁盘'">
        <div class="muted">磁盘{{ maint.low_disk ? ' ⚠ 磁盘不足，将触发紧急清理' : '' }}</div>
        <div class="num" :style="maint.low_disk ? { color: '#dc2626' } : undefined">
          {{ maint.disk_free_gb?.toFixed(1) ?? '-' }} / {{ maint.disk_total_gb?.toFixed(1) ?? '-' }}GB</div>
        <div v-if="maint.forecast_days != null && !maint.low_disk" class="muted" style="font-size:11px">
          按当前增速约 {{ maint.forecast_days }} 天（估算）</div>
      </div>
    </div>
  </AsyncState>

  <div class="panel">
    <h3 style="margin-bottom:10px">实时事件流（WebSocket）</h3>
    <table v-if="feed.length">
      <tr v-for="(e, i) in feed" :key="e.id ?? i" :class="{ 'new-item': i === 0 }">
        <td>{{ e.ts }}</td>
        <td>摄像头#{{ e.camera_id }}</td>
        <td>{{ TYPE_TEXT[e.type] || e.type }}</td>
        <td><span :class="'lv' + e.level">L{{ e.level }}</span></td>
        <td class="muted">{{ e.detail }}</td>
      </tr>
    </table>
    <p v-else-if="sys" class="muted">暂无实时事件 —— 去摄像头页添加/启动摄像头，触发行为会实时出现在这里</p>
    <p v-else class="muted">等待事件…（配置安全区与规则后，触发行为会实时出现在这里）</p>
  </div>
</template>

<style scoped>
/* 任务17B：375px 小屏适配（scoped，不动全局样式） */
.cards { flex-wrap: wrap; }
.card { min-width: 140px; }
.panel { overflow-x: auto; }
button { min-height: 40px; }
</style>
