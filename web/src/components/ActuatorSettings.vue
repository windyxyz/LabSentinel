<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { api } from '../api'

const mode = ref('disabled')
const minimumLevel = ref(2)
const url = ref('')
const port = ref('COM1')
const baud = ref(9600)
const unit = ref(1)
const coil = ref(0)
const status = ref(null)
const configured = ref(false)
const busy = ref(false)
const saved = ref(false)
const error = ref('')
let timer

const output = computed(() => {
  if (status.value?.last_error) return status.value.last_error
  if (status.value?.reconfiguring) return '正在应用联动设置'
  if (!configured.value) return '已关闭'
  const applied = status.value?.applied_level
  if (applied == null) return '等待设备确认'
  return applied === 0 ? '设备已复位' : `设备已确认 L${applied}`
})

async function load(updateForm) {
  try {
    const result = await api.alertActuator()
    status.value = result?.status ?? null
    const target = result?.config?.target
    configured.value = !!target
    if (updateForm) {
      mode.value = target?.type || 'disabled'
      minimumLevel.value = result?.config?.minimum_level ?? 2
      if (target?.type === 'http') url.value = target.url
      if (target?.type === 'modbus') {
        port.value = target.port
        baud.value = target.baud
        unit.value = target.unit
        coil.value = target.first_coil
      }
    }
  } catch (err) {
    if (updateForm) error.value = err.message
    else status.value = { last_error: '无法读取设备状态' }
  }
}

async function save() {
  busy.value = true
  error.value = ''
  saved.value = false
  try {
    let target = null
    if (mode.value === 'http') target = { type: 'http', url: url.value.trim() }
    if (mode.value === 'modbus') {
      target = { type: 'modbus', port: port.value.trim(), baud: baud.value, unit: unit.value, first_coil: coil.value }
    }
    await api.saveAlertActuator({ target, minimum_level: minimumLevel.value })
    saved.value = true
    await load(false)
  } catch (err) { error.value = err.message }
  finally { busy.value = false }
}

onMounted(async () => {
  timer = setInterval(() => { if (!busy.value) load(false) }, 2500)
  await load(true)
})
onBeforeUnmount(() => clearInterval(timer))
</script>

<template>
  <div class="actuator-options">
    <h3>声光联动</h3>
    <form class="actuator-fields" @submit.prevent="save">
      <label>设备类型
        <select v-model="mode">
          <option value="disabled">不启用</option>
          <option value="http">HTTP 继电器</option>
          <option value="modbus">USB-RS485（Modbus RTU）</option>
        </select>
      </label>
      <label v-if="mode !== 'disabled'">起始告警级别
        <select v-model.number="minimumLevel"><option :value="1">L1 提示</option><option :value="2">L2 警告</option><option :value="3">L3 严重</option></select>
      </label>
      <label v-if="mode === 'http'">继电器地址
        <input v-model="url" type="url" placeholder="http://192.168.1.20/signal" required />
      </label>
      <template v-if="mode === 'modbus'">
        <label>串口 <input v-model="port" placeholder="COM1" required /></label>
        <label>波特率
          <select v-model.number="baud">
            <option v-for="rate in [4800, 9600, 19200, 38400, 57600, 115200]" :key="rate" :value="rate">{{ rate }}</option>
          </select>
        </label>
        <label>设备地址 <input v-model.number="unit" type="number" min="1" max="247" required /></label>
        <label>首个线圈地址（从 0 起） <input v-model.number="coil" type="number" min="0" max="65533" required /></label>
      </template>
      <p v-if="mode === 'http'" class="muted">设备需接受含 <code>level</code> 的 JSON POST。0 表示复位，1 至 3 对应告警级别。</p>
      <p v-if="mode === 'modbus'" class="muted">串口使用 8E1，三个连续线圈依次对应 L1、L2、L3。</p>
      <div class="row">
        <button class="primary" type="submit" :disabled="busy">保存联动</button>
        <span role="status" :class="status?.last_error ? 'st-err' : 'muted'">{{ output }}</span>
        <span v-if="saved" class="st-ok">已保存</span>
      </div>
      <p v-if="error" role="alert">{{ error }}</p>
    </form>
  </div>
</template>

<style scoped>
.actuator-options { margin-top: 1.25rem; }
.actuator-fields { display: flex; flex-wrap: wrap; align-items: end; gap: .75rem; }
.actuator-fields label { display: grid; gap: .35rem; }
.actuator-fields p, .actuator-fields > .row { flex-basis: 100%; margin: 0; }
</style>
