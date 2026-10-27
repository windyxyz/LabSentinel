<script setup>
import { onMounted, ref } from 'vue'
import { api, adminToken } from '../api'
import WebrtcPlayer from '../components/WebrtcPlayer.vue'

const cams = ref([])
const form = ref({ name: '', source: '', type: 'file', autostart: false })
const snap = ref({})  // id -> timestamp (用于强制刷新 img)
const live = ref(null)   // { id, name, wsUrl } 正在观看实时画面的摄像头
const liveErr = ref('')
const tokenInput = ref(adminToken.get())
const tokenMsg = ref('')
// 任务23：告警渠道状态与自测
const CHANNEL_NAMES = {
  wecom: '企业微信', dingtalk: '钉钉', feishu: '飞书', serverchan: 'Server酱·微信',
  pushplus: 'PushPlus·微信', qmsg: 'Qmsg·QQ', email: '邮件',
}
const alertChannels = ref([])
const alertTesting = ref(false)
const alertTestMsg = ref('')
const failedDeliveries = ref([])

async function loadAlerts() {
  try {
    alertChannels.value = (await api.alertsStatus()).channels
    failedDeliveries.value = await api.alertDeliveries({ status: 'failed', limit: 10 })
  } catch { /* 服务未就绪时静默 */ }
}

async function runAlertTest() {
  alertTesting.value = true
  alertTestMsg.value = ''
  try {
    const results = await api.alertTest()
    const okN = results.filter(r => r.ok).length
    alertTestMsg.value = `测试完成：${okN}/${results.length} 渠道成功` +
      results.filter(r => !r.ok).map(r => `（${CHANNEL_NAMES[r.channel] || r.channel}：${r.error || '失败'}）`).join('')
    loadAlerts()
  } catch (e) { alertTestMsg.value = String(e.message || e) }
  alertTesting.value = false
}

async function retryDelivery(d) {
  try { await api.alertDeliveryRetry(d.id); loadAlerts() }
  catch (e) { alert(String(e.message || e)) }
}

function saveToken() {
  adminToken.set(tokenInput.value.trim())
  tokenMsg.value = adminToken.get() ? '令牌已保存（仅存本浏览器）' : '令牌已清除'
  refresh()
}

async function refresh() { cams.value = await api.cameras() }
onMounted(() => { refresh(); loadAlerts() })

async function add() {
  if (!form.value.name || !form.value.source) return
  await api.addCamera(form.value)
  form.value = { name: '', source: '', type: 'file', autostart: false }
  refresh()
}
async function start(id) { await api.startCamera(id); refresh() }
async function del(id) { await api.delCamera(id); refresh() }

async function watchLive(cam) {
  liveErr.value = ''
  try {
    const r = await api.cameraWebrtc(cam.id)
    live.value = { id: cam.id, name: cam.name, wsUrl: r.ws_url }
  } catch {
    liveErr.value = '获取实时画面地址失败'
  }
}
</script>

<template>
  <div class="panel">
    <h3 style="margin-bottom:10px">管理令牌</h3>
    <div class="row">
      <input v-model="tokenInput" placeholder="服务端启用鉴权时填写（LS_ADMIN_TOKEN）" aria-label="管理令牌" style="flex:1" />
      <button @click="saveToken">保存令牌</button>
      <span v-if="tokenMsg" class="muted">{{ tokenMsg }}</span>
    </div>
    <p class="muted" style="margin-top:4px">令牌仅保存在本浏览器 localStorage；服务端未启用鉴权（LS_ADMIN_TOKEN 为空）时无需填写。</p>

    <h3 style="margin:14px 0 10px">告警渠道状态</h3>
    <table>
      <tr><th>渠道</th><th>配置状态</th><th>投递 ok</th><th>failed</th><th>pending</th></tr>
      <tr v-for="ch in alertChannels" :key="ch.name">
        <td>{{ CHANNEL_NAMES[ch.name] || ch.name }}</td>
        <td>
          <span :class="'lv' + (ch.configured ? 1 : 2)">{{ ch.configured ? '已配置 ✓' : '未配置' }}</span>
        </td>
        <td>{{ ch.deliveries.ok }}</td>
        <td :style="ch.deliveries.failed ? 'color:var(--l3); font-weight:600' : ''">{{ ch.deliveries.failed }}</td>
        <td>{{ ch.deliveries.pending }}</td>
      </tr>
    </table>
    <div class="row" style="margin-top:8px">
      <button :disabled="alertTesting" @click="runAlertTest">{{ alertTesting ? '测试中…' : '发送测试告警' }}</button>
      <span v-if="alertTestMsg" class="muted">{{ alertTestMsg }}</span>
    </div>
    <p class="muted" style="margin-top:2px">渠道申请与配置方法见仓库 docs/部署指南.md「4. 告警渠道配置」。</p>

    <h3 style="margin:14px 0 10px">失败投递（最近 10 条）</h3>
    <table v-if="failedDeliveries.length">
      <tr><th>ID</th><th>事件</th><th>渠道</th><th>尝试次数</th><th>最近错误</th><th></th></tr>
      <tr v-for="d in failedDeliveries" :key="d.id">
        <td>{{ d.id }}</td>
        <td>#{{ d.event_id }}</td>
        <td>{{ CHANNEL_NAMES[d.channel] || d.channel }}</td>
        <td>{{ d.attempts }}</td>
        <td class="muted" style="max-width:280px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap">{{ d.last_error || '-' }}</td>
        <td><button class="ghost" @click="retryDelivery(d)">重发</button></td>
      </tr>
    </table>
    <p v-else class="muted">无失败投递 ✓</p>

    <h3 style="margin:14px 0 10px">添加摄像头</h3>
    <div class="row">
      <input v-model="form.name" placeholder="名称" aria-label="摄像头名称" style="width:120px" />
      <input v-model="form.source" placeholder="RTSP 地址 / 视频文件路径 / 0 (USB)" aria-label="摄像头地址" style="flex:1" />
      <select v-model="form.type">
        <option value="file">视频文件</option>
        <option value="rtsp">RTSP</option>
        <option value="usb">USB</option>
      </select>
      <label class="muted"><input type="checkbox" v-model="form.autostart" style="margin:0 4px 0 0" />立即启动</label>
      <button @click="add">添加</button>
    </div>
    <p class="muted" style="margin-top:6px">提示：不勾"立即启动"可先到"安全区配置"画好区域、加好规则，再回来启动。</p>
  </div>

  <div class="panel">
    <table>
      <tr><th>ID</th><th>名称</th><th>类型</th><th>状态</th><th>最新画面</th><th></th></tr>
      <tr v-for="c in cams" :key="c.id">
        <td>{{ c.id }}</td>
        <td>{{ c.name }}</td>
        <td>{{ c.type }}</td>
        <td>{{ c.status }}</td>
        <td><img v-if="snap[c.id]" :src="`/api/cameras/${c.id}/snapshot?t=${snap[c.id]}`" alt="摄像头最新画面缩略图" class="snap" @click="snap[c.id] = Date.now()" /></td>
        <td>
          <button class="ghost" style="margin-right:6px" @click="snap[c.id] = Date.now()">看快照</button>
          <button class="ghost" style="margin-right:6px" @click="watchLive(c)">实时画面</button>
          <button class="ghost" style="margin-right:6px" @click="start(c.id)">启动</button>
          <button class="ghost" @click="del(c.id)">删除</button>
        </td>
      </tr>
    </table>
    <p v-if="liveErr" class="muted" style="margin-top:6px">{{ liveErr }}（需先启动 tools/go2rtc.exe，见 scripts/start-all.bat）</p>
  </div>

  <div v-if="live" style="position:fixed; inset:0; background:rgba(0,0,0,.85); display:flex; align-items:center; justify-content:center; cursor:zoom-out" @click="live = null">
    <div style="width:min(92vw, 960px); cursor:default" @click.stop>
      <WebrtcPlayer :ws-url="live.wsUrl" :label="live.name" />
      <p class="muted" style="text-align:center; margin-top:8px">实时画面（WebRTC，延迟约 1s）· 点击空白处关闭</p>
    </div>
  </div>
</template>

<style scoped>
/* 任务17B：375px 小屏适配（scoped） */
.panel { overflow-x: auto; }
button { min-height: 40px; }
</style>
