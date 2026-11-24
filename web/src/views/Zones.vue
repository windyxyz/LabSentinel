<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch, nextTick } from 'vue'
import { api, mediaUrl } from '../api'
import SchedulePicker from '../components/SchedulePicker.vue'
import LabRulesPanel from '../components/LabRulesPanel.vue'

const NATIVE_W = 1280, NATIVE_H = 720
const DISP_W = 640, DISP_H = 360
const SCALE = DISP_W / NATIVE_W

const cams = ref([])
const camId = ref(null)
const zones = ref([])
const rules = ref([])
const masks = ref([])
const fpSpots = ref([])
const drawKind = ref('zone')
const maskKinds = { motion: '运动遮罩', object_filter: '目标过滤遮罩' }
const maskColors = { motion: '#38bdf8', object_filter: '#fb923c' }
const drawingBusy = ref(false)
const spatialError = ref('')
const drawing = ref([])      // 正在绘制的顶点（显示坐标）
const snapT = ref(0)
const snapshotError = ref(false)
const selectedCamera = computed(() => cams.value.find(camera => camera.id === camId.value))
const policyMessage = ref('')
const snapshotUrl = computed(() => camId.value && snapT.value
  ? mediaUrl(`/api/cameras/${camId.value}/snapshot?t=${snapT.value}${selectedCamera.value?.type === 'file' ? '&preview=1' : ''}`)
  : '')
const zname = ref('')
const newRule = ref({ behavior: 'enter', level: 2, cooldown_s: 60, time_start: '00:00', time_end: '23:59', threshold: 0,
  days_of_week: '1,2,3,4,5,6,7', holiday_only: false })  // 任务112：星期/假期条件
const newGraceSeconds = ref(0)
const newWarningPercent = ref(70)
const ruleMessage = ref('')
const graceBusy = ref(false)
const supportsGrace = (behavior) => ['enter_no_ppe', 'no_mask'].includes(behavior)
const warningPercent = (rule) => Math.round((rule.warn_ratio ?? 0.70) * 100)

async function saveRuleGrace(rule, form) {
  const settings = new FormData(form)
  graceBusy.value = true
  ruleMessage.value = ''
  try {
    await api.updateRuleGrace(rule.id, {
      grace_s: Number(settings.get('grace_s')), warn_ratio: Number(settings.get('warn_percent')) / 100,
    })
    await load()
    ruleMessage.value = '宽限已保存'
  } catch (error) {
    ruleMessage.value = error.message
  } finally {
    graceBusy.value = false
  }
}

function ruleSettings() {
  return {
    ...newRule.value,
    ...(supportsGrace(newRule.value.behavior)
      ? { grace_s: newGraceSeconds.value, warn_ratio: newWarningPercent.value / 100 }
      : {}),
  }
}
// 任务112：日期例外表（holiday=该日按假期 / workday=按工作日）
const WEEKDAYS = ['一', '二', '三', '四', '五', '六', '日']
const calExceptions = ref([])
const newCal = ref({ date: '', mode: 'holiday', note: '' })
const calMsg = ref('')

async function loadCal() {
  try {
    // 契约：/api/calendar 返回数组；兼容旧 {exceptions:[]} 包裹（2026-11-21 闪黑根因）
    const r = await api.calendar()
    calExceptions.value = Array.isArray(r) ? r : (r?.exceptions || [])
  } catch { /* 旧服务端无此端点：静默 */ }
}
async function addCal() {
  if (!newCal.value.date) { calMsg.value = '先选日期'; return }
  try {
    const r = await api.setCalendar(newCal.value)
    calMsg.value = (r.replaced ? '已覆盖 ' : '已添加 ') + newCal.value.date
    await loadCal()
  } catch (e) { calMsg.value = String(e.message || e) }
}
function dayLabel(r) {
  const s = String(r.days_of_week || '')
  if (!s || s === '1,2,3,4,5,6,7') return ''
  return '周' + s.split(',').map(x => x.trim()).filter(Boolean).map(d => WEEKDAYS[Number(d) - 1] || '?').join('')
}
// 任务80：复制区域/规则到其他摄像头
const copyTargets = ref([])          // 选中的目标摄像头 id
const copyZones = ref(true)
const copyRules = ref(true)
const copyMsg = ref('')
const copyBusy = ref(false)

onMounted(async () => {
  try {
    const cs = await api.cameras()
    cams.value = Array.isArray(cs) ? cs : []
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
      (Array.isArray(r.warnings) && r.warnings.length ? `\n${r.warnings.join('\n')}` : '')
    load()
  } catch (e) { copyMsg.value = String(e.message || e) }
  copyBusy.value = false
}

watch(camId, () => {
  snapshotError.value = false
  snapT.value = camId.value ? Date.now() : 0
  load()
  loadSpatial()
}, { immediate: true })
watch(camId, (id, _, onCleanup) => {
  policyMessage.value = ''
  if (!id) return
  let disposed = false
  let timer
  onCleanup(() => {
    disposed = true
    clearTimeout(timer)
  })
  async function checkPolicy() {
    try {
      const status = await api.cameraStatus(id)
      if (disposed || camId.value !== id) return
      const revision = status?.policy_revision
      if (!Number.isSafeInteger(revision)) {
        policyMessage.value = ''
      } else if (status.applied_policy_revision === revision) {
        policyMessage.value = '规则已生效'
      } else if (['stopped', 'completed', 'error', 'model_error'].includes(status.status)) {
        policyMessage.value = '规则已保存，启动来源后生效'
      } else {
        policyMessage.value = '正在应用规则…'
      }
    } catch {
      if (!disposed) policyMessage.value = '暂时无法确认规则状态'
    } finally {
      if (!disposed) timer = setTimeout(checkPolicy, 1500)
    }
  }
  checkPolicy()
}, { immediate: true })
onMounted(() => {
  loadCal(); loadTpls() // 任务112/116
  window.addEventListener('keydown', onKey) // LS-052 撤销/重做
})
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
// 任务116：规则模板库（只存规则参数，不绑摄像头/区域）
const tplName = ref('')
const tplList = ref([])
const tplSel = ref(null)
const tplZone = ref('')      // '' = 全局规则
const tplMsg = ref('')

// 后端模板清单不返回 rules_count（配置可能是数组或 {rules:[]}）——前端双形状兜底计算
const tplRulesCount = (t) => {
  if (Array.isArray(t?.config)) return t.config.length
  if (Array.isArray(t?.config?.rules)) return t.config.rules.length
  return t?.rules_count ?? 0
}

async function loadTpls() {
  try {
    // 契约：/api/rule-templates 返回数组；兼容旧 {templates:[]} 包裹（2026-11-21 闪黑根因）
    const r = await api.ruleTemplates()
    tplList.value = Array.isArray(r) ? r : (r?.templates || [])
  } catch { /* 旧服务端无端点 */ }
}
async function saveAsTemplate() {
  if (!tplName.value.trim()) { tplMsg.value = '先填模板名'; return }
  try {
    await api.createRuleTemplate({ name: tplName.value.trim(), rules: [ruleSettings()] })
    tplMsg.value = `模板「${tplName.value.trim()}」已保存`
    tplName.value = ''
    loadTpls()
  } catch (e) { tplMsg.value = String(e.message || e) }
}
async function applyTemplate() {
  if (!tplSel.value || !camId.value) return
  const t = tplList.value.find(x => x.id === tplSel.value)
  if (!t || !confirm(`把模板「${t.name}」（${tplRulesCount(t)} 条规则）套用到当前摄像头？`)) return
  try {
    const body = { camera_ids: [camId.value] }
    if (tplZone.value) body.zone_id = Number(tplZone.value)
    const r = await api.applyRuleTemplate(t.id, body)
    tplMsg.value = `套用完成：新建 ${r.created} / 跳过 ${r.skipped}`
    load()
  } catch (e) { tplMsg.value = String(e.message || e) }
}
async function delTemplate(id) {
  if (!confirm('删除该模板？（不影响已套用的规则）')) return
  await api.delRuleTemplate(id).then(loadTpls)
}
async function load() {
  const id = camId.value
  if (!id) return
  // 防御：接口异常形状不得让整页渲染崩溃（历史 bug：undefined.length → 闪黑/空白）
  const [zs, rs] = await Promise.all([api.zones(id), api.rules(id)])
  if (camId.value !== id) return
  zones.value = Array.isArray(zs) ? zs : []
  rules.value = Array.isArray(rs) ? rs : []
  drawing.value = []
  undoStack.value = []
  redoStack.value = []
  await nextTick()
  draw()
}

async function loadSpatial() {
  const id = camId.value
  spatialError.value = ''
  masks.value = []
  fpSpots.value = []
  if (!id) return
  try {
    const result = await api.spatialSettings(id)
    if (camId.value !== id) return
    masks.value = Array.isArray(result?.masks) ? result.masks : []
    fpSpots.value = Array.isArray(result?.spots) ? result.spots : []
    await nextTick()
    draw()
  } catch (error) {
    if (camId.value === id) spatialError.value = error.message
  }
}

async function removeSpatial(id, kind) {
  const camera = camId.value
  try {
    if (kind === 'spot') await api.deleteFpSpot(camera, id)
    else await api.deleteCameraMask(camera, id)
    if (camId.value === camera) await loadSpatial()
  } catch (error) {
    if (camId.value === camera) spatialError.value = error.message
  }
}

// LS-052：安全区高对比区分色板（浅/深主题均可见）
const ZONE_COLORS = ['#4da3ff', '#f5a524', '#22c55e', '#a855f7', '#ef4444', '#06b6d4', '#eab308', '#ec4899']
const zoneColor = (i) => ZONE_COLORS[i % ZONE_COLORS.length]

const undoStack = ref([])
const redoStack = ref([])
function pushUndo(entry = { points: drawing.value.map(p => [...p]) }) {
  undoStack.value.push(entry)
  if (undoStack.value.length > 100) undoStack.value.shift()
  redoStack.value = []
}
const editError = ref('')
const savingPolygon = ref(false)
const spatialSaving = ref(false)
const anchorModes = {
  bottom_center: '脚下',
  body_center: '身体中心',
  upper_body: '上身',
  four_point_any: '四点任一入区',
}

async function saveSpatial(zone) {
  const id = camId.value
  spatialSaving.value = true
  editError.value = ''
  try {
    await api.updateZoneSpatial(zone.id, {
      anchor_mode: zone.anchor_mode,
      margin_px: zone.margin_px,
      leave_grace_frames: zone.leave_grace_frames,
    })
    if (camId.value === id) editError.value = '区域判定已保存'
  } catch (error) {
    if (camId.value === id) editError.value = error.message
  } finally {
    spatialSaving.value = false
  }
}
async function moveHistory(from, to) {
  if (!from.value.length || savingPolygon.value) return
  const entry = from.value.at(-1)
  const zone = zones.value.find(z => z.id === entry.id)
  if (entry.id && !zone) return
  const points = (zone?.polygon || drawing.value).map(p => [...p])
  if (zone && !await persistPolygon(zone.id, entry.points)) return
  if (zone) zone.polygon = entry.points
  else drawing.value = entry.points
  from.value.pop()
  to.value.push({ id: entry.id, points })
  draw()
}
const undo = () => moveHistory(undoStack, redoStack)
const redo = () => moveHistory(redoStack, undoStack)

async function persistPolygon(id, polygon) {
  savingPolygon.value = true
  editError.value = ''
  try {
    await api.updateZone(id, polygon)
    return true
  } catch (error) {
    editError.value = error.message
    return false
  } finally {
    savingPolygon.value = false
  }
}

// LS-052：拖拽已有安全区顶点（松手保存）
const drag = ref(null) // {zid, idx, zoneIndex}
const hoverVertex = ref(null) // {zid, idx}
const SNAP_PX = 8
const dragging = ref(false)

function canvasPos(e) {
  const r = e.target.getBoundingClientRect()
  return [
    Math.round(Math.max(0, Math.min(DISP_W, (e.clientX - r.left) * DISP_W / (r.width || DISP_W)))),
    Math.round(Math.max(0, Math.min(DISP_H, (e.clientY - r.top) * DISP_H / (r.height || DISP_H)))),
  ]
}

function findVertex(pos, exclude) {
  for (let zi = 0; zi < zones.value.length; zi++) {
    const z = zones.value[zi]
    if (!Array.isArray(z.polygon)) continue
    for (let i = 0; i < z.polygon.length; i++) {
      if (!Array.isArray(z.polygon[i]) || (exclude?.zid === z.id && exclude.idx === i)) continue
      const [x, y] = z.polygon[i]
      const dx = x * SCALE - pos[0]
      const dy = y * SCALE - pos[1]
      if (Math.hypot(dx, dy) <= SNAP_PX) return { zid: z.id, idx: i, zoneIndex: zi }
    }
  }
  return null
}

function onCanvasClick(e) {
  if (dragging.value || savingPolygon.value) return
  pushUndo()
  drawing.value.push(canvasPos(e))
  draw()
}
function onMouseDown(e) {
  if (savingPolygon.value || drawKind.value !== 'zone') return
  const hit = findVertex(canvasPos(e))
  if (hit) {
    drag.value = { ...hit, original: zones.value[hit.zoneIndex].polygon.map(point => [...point]) }
    dragging.value = false
  }
}
function onMouseMove(e) {
  const pos = canvasPos(e)
  if (!drag.value) {
    hoverVertex.value = findVertex(pos)
    return
  }
  dragging.value = true
  const z = zones.value.find((x) => x.id === drag.value.zid)
  if (!z || !Array.isArray(z.polygon)) return
  const snap = findVertex(pos, drag.value)
  z.polygon[drag.value.idx] = snap ? [...zones.value[snap.zoneIndex].polygon[snap.idx]]
    : [Math.round(pos[0] / SCALE), Math.round(pos[1] / SCALE)]
  draw()
}
async function onMouseUp() {
  const edit = drag.value
  drag.value = null
  setTimeout(() => (dragging.value = false), 0)
  if (edit && dragging.value) {
    const z = zones.value.find((x) => x.id === edit.zid)
    if (z && Array.isArray(z.polygon)) {
      if (await persistPolygon(z.id, z.polygon)) pushUndo({ id: z.id, points: edit.original })
      else { z.polygon = edit.original; draw() }
    }
  }
}
async function onDblClick() {
  const id = camId.value
  const name = zname.value.trim()
  if (!id || !name || drawing.value.length < 3 || drawingBusy.value) return
  const polygon = drawing.value.map(([x, y]) => [Math.round(x / SCALE), Math.round(y / SCALE)])
    .filter(([x, y], i, points) => i === 0 || x !== points[i - 1][0] || y !== points[i - 1][1])
  if (polygon.length < 3) return
  drawingBusy.value = true
  editError.value = ''
  try {
    if (drawKind.value === 'zone') {
      await api.addZone({ camera_id: id, name, polygon })
      if (camId.value === id) await load()
    } else {
      await api.addCameraMask(id, { name, kind: drawKind.value, polygon })
      if (camId.value === id) {
        drawing.value = []
        undoStack.value = []
        redoStack.value = []
        await loadSpatial()
      }
    }
  } catch (error) {
    if (camId.value === id) editError.value = error.message
  } finally {
    drawingBusy.value = false
  }
}
function refreshBase() {
  snapshotError.value = false
  snapT.value = Date.now() // 时间戳防缓存，强制拉最新快照
  nextTick(draw)
}
function draw() {
  const c = document.getElementById('zoneCanvas')
  if (!c) return
  const ctx = c.getContext('2d')
  if (!ctx) return
  ctx.clearRect(0, 0, DISP_W, DISP_H)
  const paint = (pts, color, fill, close) => {
    if (pts.length < 2) return
    ctx.beginPath()
    ctx.moveTo(pts[0][0], pts[0][1])
    pts.slice(1).forEach(p => ctx.lineTo(p[0], p[1]))
    if (close) { ctx.closePath(); ctx.fillStyle = color + '33'; ctx.fill() }
    ctx.strokeStyle = color; ctx.lineWidth = 2; ctx.stroke()
  }
  zones.value.forEach((z, zi) => {
    if (!Array.isArray(z.polygon)) return
    const pts = z.polygon.map(([x, y]) => [x * SCALE, y * SCALE])
    const color = zoneColor(zi)
    paint(pts, color, true, true)
    // 顶点圆点 + 名称标签（居中）
    ctx.fillStyle = color
    pts.forEach(([x, y]) => { ctx.beginPath(); ctx.arc(x, y, 3.5, 0, Math.PI * 2); ctx.fill() })
    if (pts.length) {
      const cx = pts.reduce((s, p) => s + p[0], 0) / pts.length
      const cy = pts.reduce((s, p) => s + p[1], 0) / pts.length
      ctx.font = '13px sans-serif'
      ctx.fillStyle = 'rgba(0,0,0,0.55)'
      const tw = ctx.measureText(z.name || '').width + 10
      ctx.fillRect(cx - tw / 2, cy - 10, tw, 20)
      ctx.fillStyle = '#fff'
      ctx.textAlign = 'center'
      ctx.fillText(z.name || '', cx, cy + 5)
      ctx.textAlign = 'start'
    }
  })
  for (const mask of masks.value) {
    if (!Array.isArray(mask.polygon)) continue
    ctx.setLineDash([6, 4])
    paint(mask.polygon.map(([x, y]) => [x * SCALE, y * SCALE]),
      maskColors[mask.kind] || maskColors.motion, true, true)
  }
  ctx.setLineDash([])
  for (const spot of fpSpots.value) {
    if (!Array.isArray(spot.bbox) || spot.bbox.length !== 4) continue
    const [x1, y1, x2, y2] = spot.bbox
    ctx.strokeStyle = '#f472b6'
    ctx.strokeRect(x1 * SCALE, y1 * SCALE, (x2 - x1) * SCALE, (y2 - y1) * SCALE)
  }
  paint(drawing.value, '#f5c518', drawing.value.length >= 3, drawing.value.length >= 3)
  // 悬停顶点高亮
  if (hoverVertex.value) {
    const z = zones.value.find((x) => x.id === hoverVertex.value.zid)
    const v = z?.polygon?.[hoverVertex.value.idx]
    if (v) {
      ctx.beginPath()
      ctx.arc(v[0] * SCALE, v[1] * SCALE, 7, 0, Math.PI * 2)
      ctx.strokeStyle = '#fff'
      ctx.lineWidth = 2
      ctx.stroke()
    }
  }
}

// LS-052：Ctrl+Z / Ctrl+Y 撤销重做（焦点在输入框时不拦截）
function onKey(e) {
  const tag = (e.target?.tagName || '').toLowerCase()
  if (tag === 'input' || tag === 'textarea' || tag === 'select') return
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'z') { e.preventDefault(); undo() }
  else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'y') { e.preventDefault(); redo() }
}

async function addRule(zoneId) {
  ruleMessage.value = ''
  try {
    await api.addRule({ camera_id: camId.value, zone_id: zoneId, ...ruleSettings() })
    await load()
  } catch (error) {
    ruleMessage.value = error.message
  }
}
</script>

<template>
  <div class="panel">
    <div class="row">
      <b>摄像头</b>
      <select v-model="camId">
        <option v-for="c in cams" :key="c.id" :value="c.id">{{ c.name }}</option>
      </select>
      <span v-if="policyMessage" class="muted" role="status">{{ policyMessage }}</span>
      <select v-model="drawKind" aria-label="绘制类型">
        <option value="zone">安全区</option>
        <option v-for="(label, kind) in maskKinds" :key="kind" :value="kind">{{ label }}</option>
      </select>
      <input v-model="zname" :placeholder="drawKind === 'zone' ? '新安全区名称' : '新遮罩名称'" style="width:180px" aria-label="绘制名称" />
      <button class="ghost" @click="pushUndo(); drawing = []; draw()">清除未保存顶点</button>
      <button class="ghost" :disabled="!undoStack.length" title="Ctrl+Z" @click="undo">撤销</button>
      <button class="ghost" :disabled="!redoStack.length" title="Ctrl+Y" @click="redo">重做</button>
      <button class="ghost" @click="refreshBase">刷新底图</button>
      <span class="muted">单击加顶点 · 双击闭合并保存（≥3 顶点）· 拖拽已有顶点微调</span>
    </div>
    <div style="position:relative; width:640px; height:360px; background:#000; border-radius:6px">
      <img v-if="snapshotUrl && !snapshotError" :key="snapshotUrl" :src="snapshotUrl"
           alt="用于绘制安全区的底图" width="640" height="360" @error="snapshotError = true" />
      <canvas id="zoneCanvas" class="zone-canvas" width="640" height="360"
              @click="onCanvasClick" @dblclick="onDblClick"
              @mousedown="onMouseDown" @mousemove="onMouseMove" @mouseup="onMouseUp" @mouseleave="onMouseUp"></canvas>
      <p v-if="!snapshotUrl || snapshotError" class="muted" style="position:absolute; top:45%; width:100%; text-align:center; color:white" role="status">
        {{ snapshotError ? (selectedCamera?.type === 'file' ? '无法读取视频首帧，请检查文件路径后刷新底图。' : '暂无画面，请启动监控源后刷新底图。') : '请选择有可用画面的监控源。' }}
      </p>
    </div>
    <p v-if="editError" role="status">{{ editError }}</p>
  </div>

  <div v-if="camId" class="panel">
    <h3>遮罩与误报点</h3>
    <p class="muted">运动遮罩用于性能设置中的运动预筛。目标过滤遮罩用于区域进入、未穿实验服和未戴口罩告警。</p>
    <p v-if="spatialError" role="status">{{ spatialError }}</p>
    <div v-for="mask in masks" :key="'mask-' + mask.id" class="row">
      <span>{{ maskKinds[mask.kind] }} · {{ mask.name }}</span>
      <button class="ghost" @click="removeSpatial(mask.id, 'mask')">移除</button>
    </div>
    <div v-for="spot in fpSpots" :key="'spot-' + spot.id" class="row">
      <span>误报点 · 事件 #{{ spot.event_id }} · {{ spot.created_at }}</span>
      <button class="ghost" @click="removeSpatial(spot.id, 'spot')">移除</button>
    </div>
  </div>

  <LabRulesPanel v-if="camId" :camera-id="camId" :zones="zones" :snapshot="snapshotUrl" />

  <div class="panel">
    <h3 style="margin-bottom:10px">已配置安全区</h3>
    <table>
      <thead>
      <tr><th>ID</th><th>名称</th><th>顶点数</th><th>入区判定</th><th></th></tr>
      </thead>
      <tbody>
      <tr v-for="z in zones" :key="z.id">
        <td>{{ z.id }}</td><td>{{ z.name }}</td><td>{{ Array.isArray(z.polygon) ? z.polygon.length : 0 }}</td>
        <td>
          <details>
            <summary>{{ anchorModes[z.anchor_mode] || anchorModes.bottom_center }}</summary>
            <form class="zone-spatial" @submit.prevent="saveSpatial(z)">
              <label>区域锚点
                <select v-model="z.anchor_mode">
                  <option v-for="(label, value) in anchorModes" :key="value" :value="value">{{ label }}</option>
                </select>
              </label>
              <label>边界宽容（像素）
                <input v-model.number="z.margin_px" type="number" min="0" max="128" required>
              </label>
              <label>离场宽限（分析帧）
                <input v-model.number="z.leave_grace_frames" type="number" min="0" max="120" required>
              </label>
              <small>边界宽容越大，越能容忍检测框抖动。离场宽限为 0 时沿用全局设置。</small>
              <button class="ghost" :disabled="spatialSaving">保存判定</button>
            </form>
          </details>
        </td>
        <td><button class="ghost" @click="api.delZone(z.id).then(load)">删除</button></td>
      </tr>
      </tbody>
    </table>

    <h3 style="margin:14px 0 10px">告警规则</h3>
    <div class="row">
      <select v-model="newRule.behavior">
        <option value="enter_no_ppe">未穿实验服进入（白大褂）</option>
        <option value="no_mask">未戴口罩</option>
        <option value="enter">进入安全区</option>
        <option value="exit">离开安全区</option>
        <option value="occupancy_over">在场人数超限</option>
        <option value="solo_after_hours">单人留守超时（非工作时段）</option>
      </select>
      <select v-model="newRule.level"><option :value="1">L1 提示</option><option :value="2">L2 警告</option><option :value="3">L3 严重</option></select>
      <input v-model.number="newRule.cooldown_s" placeholder="冷却(秒)" style="width:90px" />
      <input v-if="['occupancy_over', 'solo_after_hours'].includes(newRule.behavior)" v-model.number="newRule.threshold"
             :placeholder="newRule.behavior === 'solo_after_hours' ? '滞留秒数' : '人数阈值'"
             style="width:100px" />
      <label class="muted"><input type="checkbox" v-model="newRule.holiday_only" />仅假期生效</label>
    </div>
    <div v-if="supportsGrace(newRule.behavior)" class="row">
      <label>宽限（秒） <input v-model.number="newGraceSeconds" type="number" min="0" max="3600" /></label>
      <label>提前提示（%） <input v-model.number="newWarningPercent" type="number" min="1" max="99" /></label>
      <span class="muted">设为 0 会立即告警</span>
    </div>
    <!-- LS-052：可视化周时段排班（100% 兼容 days_of_week/time_start/time_end 契约） -->
    <SchedulePicker
      v-model:days-of-week="newRule.days_of_week"
      v-model:time-start="newRule.time_start"
      v-model:time-end="newRule.time_end" />
    <table>
      <tr v-for="r in rules" :key="r.id">
        <td>#{{ r.id }}</td><td>区 {{ r.zone_id ?? '全部' }}</td><td>{{ r.behavior }}</td>
        <td><span :class="'lv' + r.level">L{{ r.level }}</span></td>
        <td class="muted">冷却{{ r.cooldown_s }}s · {{ r.time_start }}-{{ r.time_end }} · 阈值{{ r.threshold }}
          <span v-if="dayLabel(r)"> · {{ dayLabel(r) }}</span><span v-if="r.holiday_only"> · 仅假期</span></td>
        <td>
          <details v-if="supportsGrace(r.behavior)">
            <summary>宽限 {{ r.grace_s || 0 }} 秒</summary>
            <form class="zone-spatial" @submit.prevent="saveRuleGrace(r, $event.currentTarget)">
              <label>宽限（秒） <input name="grace_s" type="number" min="0" max="3600" :value="r.grace_s ?? 0" required /></label>
              <label>提前提示（%） <input name="warn_percent" type="number" min="1" max="99" :value="warningPercent(r)" required /></label>
              <button class="ghost" :disabled="graceBusy">保存</button>
            </form>
          </details>
          <button class="ghost" @click="api.delRule(r.id).then(load)">删除</button>
        </td>
      </tr>
    </table>
    <div class="row" style="margin-top:8px">
      <button v-for="z in zones" :key="'a' + z.id" class="ghost" @click="addRule(z.id)">给「{{ z.name }}」加此规则</button>
      <button class="ghost" @click="addRule(null)">加为全局规则</button>
    </div>
    <p v-if="ruleMessage" role="status">{{ ruleMessage }}</p>

    <!-- 任务112：日期例外表 -->
    <h3 style="margin:14px 0 10px">日期例外</h3>
    <div class="row">
      <input type="date" v-model="newCal.date" style="width:150px" />
      <select v-model="newCal.mode">
        <option value="holiday">按假期（当天可跑"仅假期"规则）</option>
        <option value="workday">按工作日（周末补班日用）</option>
      </select>
      <input v-model="newCal.note" placeholder="备注（如：国庆调休）" style="width:180px" />
      <button @click="addCal">保存</button>
    </div>
    <p v-if="calMsg" class="muted" style="margin:6px 0">{{ calMsg }}</p>
    <table v-if="calExceptions.length" style="margin-top:6px">
      <tr><th>日期</th><th>模式</th><th>备注</th><th></th></tr>
      <tr v-for="x in calExceptions" :key="x.date">
        <td>{{ x.date }}</td>
        <td>{{ x.mode === 'holiday' ? '假期' : '工作日' }}</td>
        <td class="muted">{{ x.note || '-' }}</td>
        <td><button class="ghost" @click="api.delCalendar(x.date).then(loadCal)">删除</button></td>
      </tr>
    </table>
    <p v-else class="muted" style="margin-top:6px">无例外记录：默认周末按假期（LS_WEEKEND_IS_HOLIDAY=0 可关）。</p>

    <!-- 任务116：规则模板库 -->
    <h3 style="margin:14px 0 10px">规则模板</h3>
    <div class="row">
      <select v-model="tplSel" aria-label="选择模板">
        <option :value="null">选择模板…</option>
        <option v-for="t in tplList" :key="t.id" :value="t.id">{{ t.name }}（{{ tplRulesCount(t) }} 条）</option>
      </select>
      <select v-model="tplZone" aria-label="套用目标">
        <option value="">套为全局规则</option>
        <option v-for="z in zones" :key="z.id" :value="z.id">套到区域「{{ z.name }}」</option>
      </select>
      <button @click="applyTemplate">套用到当前摄像头</button>
      <input v-model="tplName" placeholder="模板名" style="width:140px" aria-label="模板名" />
      <button class="ghost" @click="saveAsTemplate">把上方表单存为模板</button>
    </div>
    <p v-if="tplMsg" class="muted" style="margin:6px 0">{{ tplMsg }}</p>
    <details v-if="tplList.length" style="margin-top:6px">
      <summary class="muted">模板管理（{{ tplList.length }}）</summary>
      <table style="margin-top:6px">
        <tr><th>ID</th><th>名称</th><th>规则数</th><th>更新时间</th><th></th></tr>
        <tr v-for="t in tplList" :key="t.id">
          <td>{{ t.id }}</td><td>{{ t.name }}</td><td>{{ tplRulesCount(t) }}</td>
          <td class="muted">{{ t.updated_ts }}</td>
          <td><button class="ghost" @click="delTemplate(t.id)">删除</button></td>
        </tr>
      </table>
    </details>

    <h3 style="margin:14px 0 10px">复制到其他摄像头</h3>
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
.zone-spatial { display: grid; gap: 8px; max-width: 18rem; padding-block: 8px; }
.zone-spatial label { display: grid; gap: 4px; }
</style>
