<script setup>
import { computed, onMounted, ref, watch } from 'vue'
import { api } from '../api'

const options = {
  cache: { label: '缩略图缓存', days: 7, detail: '清理后缩略图会按需生成。' },
  vulkan_cache: { label: 'Vulkan 编译缓存', days: 30, detail: '清理后下次启动检测时会按需重新编译，首次启动可能稍慢。' },
  logs: { label: '历史运行日志', days: 14, detail: '保留当天正在写入的日志。' },
  events: { label: '已处置事件', days: 180, detail: '删除符合条件的事件及无其他引用的证据。保留未处置告警、进行中事件和待投递证据。' },
  normal_checks: { label: '正常图片检查副本', days: 30, detail: '清理无违规检查的派生截图与缩略图，保留原图和导入记录。' },
  dataset: { label: '标注池入库图片', days: 180, detail: '删除符合条件的池内副本与标注，包括已核验样本。可先导出训练集。' },
  backups: { label: '旧自动备份', days: 30, detail: '保留最新一份自动备份和所有手动备份。' },
}
const groupNames = {
  logs: '运行日志', thumbs: '缩略图缓存', snapshots: '截图与事件证据', videos: '视频证据',
  dataset_pool: '标注池', annotation_pool: '旧标注池', backups: '备份', database: '数据库',
  vulkan_cache: 'Vulkan 编译缓存',
}
const usage = ref(null)
const category = ref('cache')
const days = ref(options.cache.days)
const preview = ref(null)
const busy = ref(false)
const message = ref('')
const error = ref('')
const groups = computed(() => Array.isArray(usage.value?.groups) ? usage.value.groups : [])
const rules = computed(() => Array.isArray(usage.value?.policy) ? usage.value.policy : [])
const validDays = computed(() => Number.isInteger(days.value) && days.value >= 1 && days.value <= 3650)
const existingRule = computed(() => rules.value.find(rule => rule.category === category.value))

watch(category, value => { days.value = rules.value.find(rule => rule.category === value)?.days ?? options[value].days })
watch([category, days], () => { preview.value = null })

function bytes(number) {
  if (typeof number !== 'number' || !Number.isFinite(number)) return '—'
  let unit = 0
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB']
  while (number >= 1024 && unit < units.length - 1) { number /= 1024; unit++ }
  return `${number.toFixed(unit ? 1 : 0)} ${units[unit]}`
}

async function run(action) {
  if (busy.value) return
  busy.value = true
  message.value = ''
  error.value = ''
  try { await action() }
  catch (err) { error.value = err.message || String(err) }
  finally { busy.value = false }
}
async function refresh() { usage.value = await api.storageUsage() }
function selection() { return { category: category.value, days: days.value } }
function scan() {
  preview.value = null
  return run(async () => {
    const result = await api.storagePreview(selection())
    if (typeof result?.preview !== 'string') throw new Error('清理预览返回无效')
    preview.value = result
  })
}
function clean() {
  const token = preview.value?.preview
  if (!token) return
  preview.value = null
  return run(async () => {
    const result = await api.storageCleanup({ selection: selection(), preview: token })
    message.value = `已处理 ${result.records} 条记录，删除 ${result.files} 个文件，释放 ${bytes(result.freed_bytes)}。`
      + (result.skipped_files ? `有 ${result.skipped_files} 个文件因变更或占用未删除。` : '')
    await refresh()
  })
}
function savePolicy(enable) {
  return run(async () => {
    const updated = rules.value.filter(rule => rule.category !== category.value)
    if (enable) updated.push(selection())
    await api.storagePolicy(updated)
    await refresh()
    message.value = enable ? '已保存自动清理规则。' : '已关闭该类自动清理。'
  })
}
function compactDatabase() {
  if (!window.confirm('整理数据库期间写入可能短暂等待。现在整理吗？')) return
  return run(async () => {
    const result = await api.storageCompact()
    message.value = `已整理数据库，释放 ${bytes(result.reclaimed_bytes)}。`
    await refresh()
  })
}
onMounted(() => run(async () => {
  await refresh()
  if (existingRule.value) days.value = existingRule.value.days
}))
</script>

<template>
  <h3>存储管理</h3>
  <div class="row">
    <strong>可用 {{ bytes(usage?.free_bytes) }}</strong>
    <span class="muted">总容量 {{ bytes(usage?.total_bytes) }}</span>
    <button class="ghost" :disabled="busy" @click="run(refresh)">刷新占用</button>
  </div>
  <table v-if="groups.length">
    <thead><tr><th>数据</th><th>文件数</th><th>占用空间</th></tr></thead>
    <tbody><tr v-for="group in groups" :key="group.id">
      <td>{{ groupNames[group.id] || group.id }}</td><td>{{ group.files }}</td><td>{{ bytes(group.bytes) }}</td>
    </tr></tbody>
  </table>
  <div class="row" v-if="usage?.database_reusable_bytes > 0">
    <span class="muted">数据库内部可复用 {{ bytes(usage.database_reusable_bytes) }}</span>
    <button class="ghost" :disabled="busy" @click="compactDatabase">整理数据库</button>
  </div>
  <div class="row storage-controls">
    <label>清理类别
      <select v-model="category" :disabled="busy">
        <option v-for="(option, id) in options" :key="id" :value="id">{{ option.label }}</option>
      </select>
    </label>
    <label>保留天数
      <input v-model.number="days" type="number" min="1" max="3650" :disabled="busy" />
    </label>
    <button class="ghost" :disabled="busy || !validDays" @click="scan">预览本批</button>
  </div>
  <p class="muted">{{ options[category].detail }}</p>
  <div v-if="preview" class="row" role="status">
    <span>本批 {{ preview.records }} 条记录、{{ preview.files }} 个文件，预计释放 {{ bytes(preview.bytes) }}。</span>
    <button v-if="preview.records || preview.files" :disabled="busy" @click="clean">确认清理本批</button>
  </div>
  <div class="row storage-controls">
    <span class="muted">{{ existingRule ? `自动保留 ${existingRule.days} 天` : '自动清理关闭' }}</span>
    <button class="ghost" :disabled="busy || !validDays" @click="savePolicy(true)">保存自动清理</button>
    <button v-if="existingRule" class="ghost" :disabled="busy" @click="savePolicy(false)">关闭自动清理</button>
  </div>
  <p v-if="error" role="alert">{{ error }}</p>
  <p v-if="message" role="status">{{ message }}</p>
</template>

<style scoped>
.storage-controls { flex-wrap: wrap; align-items: end; gap: 10px; margin-top: 12px; }
.storage-controls label { display: grid; gap: 4px; }
.storage-controls input { width: 7rem; }
table { margin-block: 12px; }
</style>
