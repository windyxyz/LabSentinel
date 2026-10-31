<script setup>
import { onMounted, ref, watch, nextTick } from 'vue'
import { api, mediaUrl } from '../api'

const NATIVE_W = 1280, NATIVE_H = 720
const DISP_W = 640, DISP_H = 360
const SCALE = DISP_W / NATIVE_W

const cams = ref([])
const camId = ref(null)
const zones = ref([])
const rules = ref([])
const drawing = ref([])      // 正在绘制的顶点（显示坐标）
const snapT = ref(0)
const zname = ref('')
const newRule = ref({ behavior: 'enter', level: 2, cooldown_s: 60, time_start: '00:00', time_end: '23:59', threshold: 0 })
// 任务80：复制区域/规则到其他摄像头
const copyTargets = ref([])          // 选中的目标摄像头 id
const copyZones = ref(true)
const copyRules = ref(true)
const copyMsg = ref('')
const copyBusy = ref(false)

onMounted(async () => {
  try {
    cams.value = await api.cameras()
  } catch { /* 服务未就绪时留空，可刷新重试 */ }
  if (cams.value.length) camId.value = cams.value[0].id
})

async function doCopy() {
  if (!camId.value || !copyTargets.value.length || copyBusy.value) return
  if (!confirm(`把「${cams.value.find(c => c.id === camId.value)?.name}」的区域/规则复制到 ` +
      `${copyTargets.value.length} 路？同名区域跳过；规则按区域名映射（不会指向源摄像头的区域）。`)) return
  copyBusy.value = true
  copyMsg.value = '复制中…'
  try {
    const r = await api.copyConfig(camId.value, {
      target_ids: copyTargets.value, copy_zones: copyZones.value,
      copy_rules: copyRules.value, name_suffix: '' })
    copyMsg.value = `复制完成：区域 新建${r.zones_created}/跳过${r.zones_skipped}；` +
      `规则 新建${r.rules_created}/跳过${r.rules_skipped}` +
      (r.warnings.length ? `\n${r.warnings.join('\n')}` : '')
    load()
  } catch (e) { copyMsg.value = String(e.message || e) }
  copyBusy.value = false
}

watch(camId, load, { immediate: true })
async function load() {
  if (!camId.value) return
  zones.value = await api.zones(camId.value)
  rules.value = await api.rules(camId.value)
  drawing.value = []
  await nextTick()
  draw()
}

function onCanvasClick(e) {
  const r = e.target.getBoundingClientRect()
  drawing.value.push([Math.round(e.clientX - r.left), Math.round(e.clientY - r.top)])
  draw()
}
function onDblClick() {
  if (drawing.value.length >= 3 && zname.value) {
    const polygon = drawing.value.map(([x, y]) => [Math.round(x / SCALE), Math.round(y / SCALE)])
    api.addZone({ camera_id: camId.value, name: zname.value, polygon }).then(load)
  }
}
function draw() {
  const c = document.getElementById('zoneCanvas')
  if (!c) return
  const ctx = c.getContext('2d')
  ctx.clearRect(0, 0, DISP_W, DISP_H)
  const paint = (pts, color, fill) => {
    if (pts.length < 2) return
    ctx.beginPath()
    ctx.moveTo(pts[0][0], pts[0][1])
    pts.slice(1).forEach(p => ctx.lineTo(p[0], p[1]))
    if (fill) { ctx.closePath(); ctx.fillStyle = color + '33'; ctx.fill() }
    ctx.strokeStyle = color; ctx.lineWidth = 2; ctx.stroke()
  }
  zones.value.forEach(z => paint(z.polygon.map(([x, y]) => [x * SCALE, y * SCALE]), '#4da3ff', true))
  paint(drawing.value, '#f5c518', drawing.value.length >= 3)
}

async function addRule(zoneId) {
  await api.addRule({ camera_id: camId.value, zone_id: zoneId, ...newRule.value })
  load()
}
</script>

<template>
  <div class="panel">
    <div class="row">
      <b>摄像头</b>
      <select v-model="camId">
        <option v-for="c in cams" :key="c.id" :value="c.id">{{ c.name }}</option>
      </select>
      <input v-model="zname" placeholder="新安全区名称（先填再画）" style="width:180px" />
      <button class="ghost" @click="drawing = []; draw()">清除未保存顶点</button>
      <span class="muted">单击加顶点 · 双击闭合并保存（≥3 顶点）</span>
    </div>
    <div style="position:relative; width:640px; height:360px; background:#000; border-radius:6px">
      <img v-if="camId" :src="mediaUrl(`/api/cameras/${camId}/snapshot?t=${snapT}`)" alt="摄像头画面（用于绘制安全区）" width="640" height="360" @error="snapT = 0" />
      <canvas id="zoneCanvas" class="zone-canvas" width="640" height="360" @click="onCanvasClick" @dblclick="onDblClick"></canvas>
      <p v-if="!snapT" class="muted" style="position:absolute; top:45%; width:100%; text-align:center">
        点击"取最新快照"显示画面后再画区
      </p>
    </div>
    <div class="row" style="margin-top:10px">
      <button class="ghost" @click="snapT = Date.now(); nextTick(draw)">取最新快照</button>
    </div>
  </div>

  <div class="panel">
    <h3 style="margin-bottom:10px">已配置安全区</h3>
    <table>
      <tr><th>ID</th><th>名称</th><th>顶点数</th><th></th></tr>
      <tr v-for="z in zones" :key="z.id">
        <td>{{ z.id }}</td><td>{{ z.name }}</td><td>{{ z.polygon.length }}</td>
        <td><button class="ghost" @click="api.delZone(z.id).then(load)">删除</button></td>
      </tr>
    </table>

    <h3 style="margin:14px 0 10px">告警规则</h3>
    <div class="row">
      <select v-model="newRule.behavior">
        <option value="enter_no_ppe">进入未穿实验服</option>
        <option value="enter">进入安全区</option>
        <option value="exit">离开安全区</option>
        <option value="occupancy_over">在场人数超限</option>
      </select>
      <select v-model="newRule.level"><option :value="1">L1 提示</option><option :value="2">L2 警告</option><option :value="3">L3 严重</option></select>
      <input v-model.number="newRule.cooldown_s" placeholder="冷却(秒)" style="width:90px" />
      <input v-model="newRule.time_start" style="width:70px" /> —
      <input v-model="newRule.time_end" style="width:70px" />
      <input v-model.number="newRule.threshold" placeholder="人数阈值" style="width:90px" />
    </div>
    <table>
      <tr v-for="r in rules" :key="r.id">
        <td>#{{ r.id }}</td><td>区 {{ r.zone_id ?? '全部' }}</td><td>{{ r.behavior }}</td>
        <td><span :class="'lv' + r.level">L{{ r.level }}</span></td>
        <td class="muted">冷却{{ r.cooldown_s }}s · {{ r.time_start }}-{{ r.time_end }} · 阈值{{ r.threshold }}</td>
        <td><button class="ghost" @click="api.delRule(r.id).then(load)">删除</button></td>
      </tr>
    </table>
    <div class="row" style="margin-top:8px">
      <button v-for="z in zones" :key="'a' + z.id" class="ghost" @click="addRule(z.id)">给「{{ z.name }}」加此规则</button>
      <button class="ghost" @click="addRule(null)">加为全局规则（人数超限用）</button>
    </div>

    <h3 style="margin:14px 0 10px">复制到其他摄像头（任务80）</h3>
    <div v-if="cams.length <= 1" class="muted">只有一路摄像头——先在「摄像头」页添加更多，再来复制配置。</div>
    <template v-else>
      <div class="row" style="align-items:flex-start">
        <b>复制到：</b>
        <span style="display:flex; flex-direction:column; gap:2px">
          <label v-for="c in cams.filter(c => c.id !== camId)" :key="c.id" class="muted">
            <input type="checkbox" :value="c.id" v-model="copyTargets" style="margin:0 4px 0 0" />
            {{ c.name }}
          </label>
        </span>
        <label class="muted"><input type="checkbox" v-model="copyZones" />复制区域</label>
        <label class="muted"><input type="checkbox" v-model="copyRules" />复制规则</label>
        <button :disabled="copyBusy || !copyTargets.length" @click="doCopy">
          {{ copyBusy ? '复制中…' : `复制到 ${copyTargets.length} 路` }}</button>
      </div>
      <p v-if="copyMsg" class="muted" style="margin-top:6px; white-space:pre-wrap">{{ copyMsg }}</p>
      <p class="muted" style="margin-top:4px">
        规则按区域名映射到目标摄像头（不会指向源摄像头的区域）；同名区域跳过。</p>
    </template>
  </div>
</template>

<style scoped>
/* 任务17B：375px 小屏适配（scoped） */
.panel { overflow-x: auto; }
button { min-height: 40px; }
</style>
