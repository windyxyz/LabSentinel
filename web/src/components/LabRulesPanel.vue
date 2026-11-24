<script setup>
import { computed, nextTick, ref, watch } from 'vue'
import { api } from '../api'

const props = defineProps({
  cameraId: { type: Number, default: null },
  zones: { type: Array, default: () => [] },
  snapshot: { type: String, default: '' },
})

const lineKinds = { hood_lean: '通风橱探身线', sash_limit: '拉门高度线' }
const checkKinds = { incubator_door_open: '培养箱门', spill_or_leak: '泼洒或泄漏', glove_contamination: '手套疑似交叉接触' }
const gear = { coat: '白大褂', mask: '口罩', gloves: '手套', goggles: '护目镜' }
const empty = () => ({ reference_lines: [], ppe_policies: [], fixed_checks: [] })
const rules = ref(empty())
const expanded = ref(false)
const loaded = ref(false)
const busy = ref(false)
const message = ref('')
const canvas = ref(null)
const image = ref(null)
const pending = ref(null)
const selectedLine = ref('hood_lean')
const selectedCheck = ref('incubator_door_open')
const selectedZone = ref(null)
let generation = 0

const unusedChecks = computed(() => Object.keys(checkKinds).filter(kind => !rules.value.fixed_checks.some(check => check.kind === kind)))
watch(unusedChecks, kinds => { if (!kinds.includes(selectedCheck.value)) selectedCheck.value = kinds[0] || '' })

async function load() {
  const request = ++generation
  loaded.value = false
  rules.value = empty()
  pending.value = null
  message.value = ''
  if (!expanded.value || props.cameraId == null) return
  try {
    const result = await api.cameraLabRules(props.cameraId)
    if (request !== generation) return
    if (!['reference_lines', 'ppe_policies', 'fixed_checks'].every(key => Array.isArray(result?.[key]))) {
      throw new Error('读取到的实验室规则格式无效')
    }
    rules.value = result
    loaded.value = true
    await nextTick()
    paint()
  } catch (error) {
    if (request === generation) message.value = error.message
  }
}

watch(() => props.cameraId, () => { selectedZone.value = null; load() })

function startDrawing(type, kind) {
  pending.value = { type, kind, point: null }
  message.value = type === 'line' ? '在画面上依次点选两个端点。危险侧为从起点看向终点的右侧。' : '在画面上点击检查区域的两个对角。'
  nextTick(paint)
}

function choosePoint(event) {
  if (!pending.value || !image.value) return
  const box = image.value.getBoundingClientRect()
  if (!box.width || !box.height) return
  const coordinate = (pixels, scale) => Math.max(0, Math.min(1, Math.round(pixels / scale * 10000) / 10000))
  const point = [coordinate(event.clientX - box.left, box.width), coordinate(event.clientY - box.top, box.height)]
  const action = pending.value
  if (!action.point) {
    action.point = point
    paint()
    return
  }
  const [x1, y1] = action.point
  const [x2, y2] = point
  if (action.type === 'line') {
    if (x1 === x2 && y1 === y2) { message.value = '请选择两个不同的端点'; return }
    const base = lineKinds[action.kind]
    let name = base
    let suffix = 2
    while (rules.value.reference_lines.some(line => line.name === name)) name = `${base} ${suffix++}`
    rules.value.reference_lines.push({ name, kind: action.kind, zone_id: null, points: [action.point, point] })
  } else {
    if (x1 === x2 || y1 === y2) { message.value = '请框选具有宽度和高度的区域'; return }
    rules.value.fixed_checks.push({ kind: action.kind, roi: [Math.min(x1, x2), Math.min(y1, y2), Math.max(x1, x2), Math.max(y1, y2)], hold_s: 4 })
  }
  pending.value = null
  message.value = ''
  paint()
}

function paint() {
  const target = canvas.value
  if (!target) return
  const ctx = target.getContext('2d')
  if (!ctx) return
  const [w, h] = [target.width, target.height]
  ctx.clearRect(0, 0, w, h)
  ctx.lineWidth = 3
  for (const rule of rules.value.reference_lines) {
    const [[x1, y1], [x2, y2]] = rule.points
    ctx.strokeStyle = '#fbbf24'
    ctx.beginPath()
    ctx.moveTo(x1 * w, y1 * h)
    ctx.lineTo(x2 * w, y2 * h)
    ctx.stroke()
    ctx.fillStyle = '#fbbf24'
    ctx.beginPath()
    ctx.arc(x1 * w, y1 * h, 5, 0, Math.PI * 2)
    ctx.fill()
  }
  ctx.strokeStyle = '#38bdf8'
  for (const rule of rules.value.fixed_checks) {
    const [x1, y1, x2, y2] = rule.roi
    ctx.strokeRect(x1 * w, y1 * h, (x2 - x1) * w, (y2 - y1) * h)
  }
  if (pending.value?.point) {
    const [x, y] = pending.value.point
    ctx.fillStyle = '#ffffff'
    ctx.fillRect(x * w - 3, y * h - 3, 6, 6)
  }
}

function addPpe() {
  if (rules.value.ppe_policies.some(policy => policy.zone_id === selectedZone.value)) {
    message.value = '这个范围已经有穿戴策略'
    return
  }
  rules.value.ppe_policies.push({ zone_id: selectedZone.value, alternatives: [['coat', 'mask']] })
  message.value = ''
}

function remove(items, index) { items.splice(index, 1); paint() }

async function save() {
  const request = generation
  busy.value = true
  message.value = ''
  try {
    await api.saveCameraLabRules(props.cameraId, rules.value)
    if (generation === request) message.value = '已保存，当前监控源会使用新规则'
  } catch (error) {
    if (generation === request) message.value = error.message
  } finally { busy.value = false }
}
</script>

<template>
  <details class="card lab-rules" @toggle="expanded = $event.target.open; load()">
    <summary>实验室专项规则</summary>
    <p v-if="message" class="muted" role="status">{{ message }}</p>
    <template v-if="loaded">
      <div class="lab-controls">
        <label>基准线 <select v-model="selectedLine"><option v-for="(label, key) in lineKinds" :key="key" :value="key">{{ label }}</option></select></label>
        <button type="button" :disabled="!snapshot" @click="startDrawing('line', selectedLine)">在画面标线</button>
        <label>固定机位检查 <select v-model="selectedCheck"><option v-for="key in unusedChecks" :key="key" :value="key">{{ checkKinds[key] }}</option></select></label>
        <button type="button" :disabled="!snapshot || !selectedCheck" @click="startDrawing('fixed', selectedCheck)">框选区域</button>
        <button v-if="pending" type="button" class="ghost" @click="pending = null; message = ''; paint()">取消绘制</button>
      </div>
      <div v-if="snapshot && (pending || rules.reference_lines.length || rules.fixed_checks.length)" class="lab-stage" :class="{ drawing: pending }" @click="choosePoint">
        <img ref="image" :src="snapshot" alt="用于标定实验室规则的画面" @load="paint" />
        <canvas ref="canvas" width="640" height="360" aria-hidden="true" />
      </div>
      <div v-for="(line, index) in rules.reference_lines" :key="index" class="lab-controls lab-entry">
        <input v-model="line.name" aria-label="基准线名称" />
        <select v-model="line.zone_id" aria-label="基准线范围"><option :value="null">整个画面</option><option v-for="zone in zones" :key="zone.id" :value="zone.id">{{ zone.name }}</option></select>
        <button type="button" class="ghost" @click="line.points.reverse(); paint()">反转危险侧</button>
        <button type="button" class="ghost" @click="remove(rules.reference_lines, index)">移除</button>
      </div>
      <div v-for="(check, index) in rules.fixed_checks" :key="check.kind" class="lab-controls lab-entry">
        <strong>{{ checkKinds[check.kind] }}</strong>
        <label>持续秒数 <input v-model.number="check.hold_s" type="number" min="1" step="1" /></label>
        <button type="button" class="ghost" @click="remove(rules.fixed_checks, index)">移除</button>
      </div>

      <div class="lab-controls lab-entry">
        <label>穿戴范围 <select v-model="selectedZone"><option :value="null">整个画面</option><option v-for="zone in zones" :key="zone.id" :value="zone.id">{{ zone.name }}</option></select></label>
        <button type="button" @click="addPpe">添加穿戴策略</button>
      </div>
      <fieldset v-for="(policy, index) in rules.ppe_policies" :key="policy.zone_id ?? 'all'">
        <legend>{{ zones.find(zone => zone.id === policy.zone_id)?.name || '整个画面' }}</legend>
        <p class="muted">满足任意一组即可。同组所选装备均须穿戴。</p>
        <div v-for="(alternative, group) in policy.alternatives" :key="group" class="lab-controls">
          <span v-if="group > 0">或</span>
          <label v-for="(label, key) in gear" :key="key"><input v-model="policy.alternatives[group]" type="checkbox" :value="key" /> {{ label }}</label>
          <button v-if="policy.alternatives.length > 1" type="button" class="ghost" @click="policy.alternatives.splice(group, 1)">移除本组</button>
        </div>
        <button type="button" class="ghost" @click="policy.alternatives.push([])">添加一组</button>
        <button type="button" class="ghost" @click="rules.ppe_policies.splice(index, 1)">移除策略</button>
      </fieldset>
      <button type="button" :disabled="busy || pending != null" @click="save">{{ busy ? '正在保存…' : '保存专项规则' }}</button>
    </template>
  </details>
</template>

<style scoped>
.lab-rules { margin-block: 16px; }
summary { cursor: pointer; font-weight: 600; }
.lab-controls { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 12px; margin-block: 10px; }
.lab-controls label { display: inline-flex; align-items: center; gap: 6px; }
.lab-entry { padding-top: 8px; border-top: 1px solid var(--border); }
.lab-stage { position: relative; max-width: 640px; }
.lab-stage img { display: block; width: 100%; height: auto; }
.lab-stage.drawing { cursor: crosshair; }
canvas { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
fieldset { margin: 8px 0 12px; border: 1px solid var(--border); border-radius: 6px; }
input[type='number'] { width: 90px; }
</style>
