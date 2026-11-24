<script setup>
import { onBeforeUnmount, ref, watch } from 'vue'
import { api, TYPE_TEXT } from '../api'

const props = defineProps({ cameraId: { type: [Number, String], default: '' } })
const rows = ref([])
const opened = ref(false)
const busy = ref(false)
const message = ref('')
const previewId = ref(null)
const preview = ref('')
const saved = ref({})
const uploading = ref(null)
const PAGE = 25
let request = 0
let live = true
const labels = {
  remembered_spot: '记住的误报点', filter_mask: '目标过滤遮罩', motion_mask: '运动遮罩',
  stationary: '静止目标降频', grace_recovered: '宽限期内恢复',
}
function clearPreview() {
  previewId.value = null
  if (preview.value) URL.revokeObjectURL(preview.value)
  preview.value = ''
}
async function load(append = false) {
  const run = ++request
  if (!append) {
    rows.value = []
    clearPreview()
  }
  busy.value = true
  message.value = ''
  try {
    const result = await api.suppressions({
      limit: PAGE,
      ...(props.cameraId ? { camera_id: props.cameraId } : {}),
      ...(append && rows.value.length ? { before_id: rows.value.at(-1).id } : {}),
    })
    if (run !== request) return
    if (!Array.isArray(result)) throw new Error('抑制记录暂时不可用')
    rows.value = [...rows.value, ...result]
    more.value = result.length === PAGE
  } catch (error) {
    if (run === request) message.value = error.message
  } finally {
    if (run === request) busy.value = false
  }
}
const more = ref(false)
watch(() => props.cameraId, () => { if (opened.value) load() })
function toggle(open) {
  opened.value = open
  if (open) load()
  else {
    request += 1
    more.value = false
    clearPreview()
  }
}
async function showPreview(id) {
  if (previewId.value === id) { clearPreview(); return }
  clearPreview()
  previewId.value = id
  message.value = ''
  try {
    const blob = await api.suppressionSnapshot(id)
    if (live && previewId.value === id) preview.value = URL.createObjectURL(blob)
  } catch (error) {
    if (previewId.value === id) {
      clearPreview()
      message.value = error.message
    }
  }
}
async function addSample(id) {
  if (uploading.value !== null) return
  uploading.value = id
  message.value = ''
  try {
    const result = await api.saveSuppressionSample(id)
    if (!result?.sample?.id) throw new Error('未取得样本编号')
    saved.value = { ...saved.value, [id]: result.sample.id }
  } catch (error) {
    message.value = error.message
  } finally {
    uploading.value = null
  }
}
onBeforeUnmount(() => { live = false; request += 1; clearPreview() })
</script>

<template>
  <details class="suppression-review" @toggle="toggle($event.target.open)">
    <summary>抑制审视</summary>
    <div class="row review-controls">
      <button type="button" class="ghost" :disabled="busy" @click="load()">刷新</button>
      <p v-if="message" role="status">{{ message }}</p>
      <p v-else-if="busy && !rows.length" role="status">读取抑制记录…</p>
      <p v-else-if="!rows.length">当前范围暂无抑制记录。</p>
    </div>
    <ul v-if="rows.length">
      <li v-for="record in rows" :key="record.id">
        <div class="row">
          <b>{{ labels[record.reason] || record.reason }}</b>
          <span class="muted">{{ record.ts }} · 来源 #{{ record.camera_id }} · {{ record.hits }} 次</span>
          <span v-if="record.behavior !== 'person_inference'">{{ TYPE_TEXT[record.behavior] || record.behavior }}</span>
          <button v-if="record.has_snapshot" class="ghost" type="button" @click="showPreview(record.id)">
            {{ previewId === record.id ? '收起截图' : '看截图' }}
          </button>
          <button v-if="record.has_snapshot" class="ghost" type="button" :disabled="uploading !== null || Boolean(saved[record.id])" @click="addSample(record.id)">
            {{ saved[record.id] ? '已加入 #' + saved[record.id] : uploading === record.id ? '加入中…' : '加入标注池' }}
          </button>
        </div>
        <img v-if="preview && previewId === record.id" :src="preview" alt="触发这条抑制记录的画面" />
      </li>
    </ul>
    <button v-if="more" type="button" class="ghost" :disabled="busy" @click="load(true)">加载更早记录</button>
  </details>
</template>

<style scoped>
.suppression-review { border-top: 1px solid var(--border); padding-block: 10px; }
summary { cursor: pointer; font-weight: 600; }
.review-controls { margin-block: 8px; }
ul { padding: 0; list-style: none; }
li { border-bottom: 1px solid var(--border); padding-block: 8px; }
img { display: block; margin-top: 8px; max-width: min(100%, 480px); border-radius: var(--overlay-radius); }
</style>
