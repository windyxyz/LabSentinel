<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api, mediaUrl, TYPE_TEXT } from '../api'
import AsyncState from '../components/AsyncState.vue'

const folders = ref([])
const selected = ref('')
const days = ref(7)
const filter = ref('key')          // key=有人 | violation=仅违规 | all=全部
const items = ref([])
const summary = ref({ items: [], totals: {}, days: 7 })
const live = ref({})               // camera_id -> manager 运行指标
const loading = ref(true)
const loaded = ref(false)
const error = ref('')
const notice = ref('')
const detail = ref(null)
const LIMIT = 60
let timer = null

const currentSummary = computed(() =>
  summary.value.items.find(x => String(x.camera_id) === String(selected.value)) || null)
const currentLive = computed(() => selected.value ? live.value[selected.value] : null)
const total = computed(() => summary.value.totals || {})
const dayLabel = computed(() => days.value >= 3650 ? '不限时间' : `近 ${days.value} 天`)

function localIso(d) {
  const p = n => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T` +
    `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
}

function query() {
  const q = { limit: LIMIT, offset: 0 }
  if (selected.value) q.camera_id = selected.value
  if (filter.value === 'key') q.key = 1
  if (filter.value === 'violation') q.violation = 1
  if (days.value < 3650) q.start_ts = localIso(new Date(Date.now() - days.value * 86400000))
  return q
}

function exportUrl() {
  const q = query()
  delete q.limit
  delete q.offset
  const qs = new URLSearchParams(q).toString()
  return '/api/folder-images/export.csv' + (qs ? '?' + qs : '')
}

async function reload(showSpinner = false) {
  if (showSpinner) loading.value = true
  try {
    const [cams, sum, sys, pics] = await Promise.all([
      api.cameras(),
      api.folderImagesSummary({ days: days.value, ...(selected.value ? { camera_id: selected.value } : {}) }),
      api.system(),
      api.folderImages(query()),
    ])
    folders.value = cams.filter(c => c.type === 'folder')
    summary.value = sum
    items.value = pics.items
    live.value = Object.fromEntries((sys.cameras || [])
      .filter(c => c.kind === 'folder').map(c => [String(c.id), c]))
    if (!selected.value && folders.value.length) {
      // 首次进入默认选第一个文件夹（保持"打开即有内容"）
      selected.value = String(folders.value[0].id)
      await reload(false)
      return
    }
    error.value = ''
    notice.value = ''
    loaded.value = true
  } catch (e) {
    if (loaded.value) notice.value = '刷新失败，仍显示上一次结果（自动重试中）'
    else error.value = String(e.message || e)
  } finally {
    loading.value = false
  }
}

function pickCamera() {
  detail.value = null
  reload(true)
}

onMounted(() => {
  reload(true)
  timer = setInterval(() => { if (!document.hidden) reload(false) }, 15000)
})
onUnmounted(() => clearInterval(timer))

function violationsText(it) {
  return (it.violations || []).map(v =>
    `${TYPE_TEXT[v.type] || v.type}${v.zone ? `「${v.zone}」` : ''}`).join('、')
}

function thumbFailed(it) {
  it.thumb_error = true
}
</script>

<template>
  <div class="panel">
    <div class="row" style="justify-content:space-between; align-items:center">
      <h3 style="margin:0">图片识别 · 文件夹接入</h3>
      <div class="row" style="align-items:center">
        <select v-model="selected" aria-label="选择图片文件夹" @change="pickCamera">
          <option value="">全部文件夹</option>
          <option v-for="f in folders" :key="f.id" :value="String(f.id)">{{ f.name }}（#{{ f.id }}）</option>
        </select>
        <select v-model.number="days" aria-label="时间范围" @change="reload(true)">
          <option :value="1">近 24 小时</option>
          <option :value="7">近 7 天</option>
          <option :value="30">近 30 天</option>
          <option :value="3650">不限时间</option>
        </select>
        <select v-model="filter" aria-label="图片筛选" @change="reload(true)">
          <option value="key">关键图片（有人）</option>
          <option value="violation">仅违规</option>
          <option value="all">全部已识别</option>
        </select>
        <button class="ghost" @click="reload(true)">刷新</button>
        <a class="link-btn" :href="mediaUrl(exportUrl())">导出 CSV</a>
      </div>
    </div>
    <p class="muted" style="margin:6px 0 10px">
      外部截图软件把图片写入「摄像头」页登记的文件夹后，系统自动识别人员/实验服，并按安全区规则汇报。
      原始截图只读；列表每 15 秒自动刷新。</p>

    <div class="cards">
      <div class="card"><div class="muted">接入文件夹</div><div class="num">{{ folders.length }}</div></div>
      <div class="card"><div class="muted">{{ dayLabel }}识别</div><div class="num">{{ total.total || 0 }}</div></div>
      <div class="card">
        <div class="muted">关键图片（有人）</div>
        <div class="num" :style="total.key ? 'color:#b45309' : undefined">{{ total.key || 0 }}</div>
      </div>
      <div class="card">
        <div class="muted">违规图片</div>
        <div class="num" :style="total.violation ? 'color:var(--l3)' : undefined">{{ total.violation || 0 }}</div>
      </div>
      <div class="card">
        <div class="muted">读取失败</div>
        <div class="num" :style="total.errors ? 'color:#dc2626' : undefined">{{ total.errors || 0 }}</div>
      </div>
      <div v-if="currentLive" class="card">
        <div class="muted">当前文件夹（{{ currentLive.status }}）</div>
        <div class="num" style="font-size:18px; padding-top:6px">
          待处理 {{ currentLive.backlog ?? 0 }}</div>
        <div class="muted" style="font-size:11px">
          本轮已识别 {{ currentLive.processed_total ?? 0 }} 张
          <span v-if="currentLive.error_total"> · 失败 {{ currentLive.error_total }}</span>
        </div>
      </div>
    </div>

    <div v-if="!loading && !folders.length" class="panel" style="margin-top:12px">
      <p class="muted" style="margin:0">
        还没有图片文件夹源：到「摄像头」页，类型选「图片文件夹」，填入截图目录路径并勾选“立即启动”。
        建议先到「安全区配置」画好区域、加好规则，识别结果才会产生违规事件与告警。</p>
    </div>

    <AsyncState :loading="loading" :error="error" :notice="notice"
                :empty="loaded && !items.length"
                empty-text="当前范围内没有符合条件的图片 —— 等待截图软件写入新图，或切换为「全部已识别」/更长时间范围"
                @retry="reload(true)">
      <div class="grid">
        <div v-for="it in items" :key="it.id" class="shot" @click="detail = it">
          <div class="thumb-wrap">
            <img v-if="!it.thumb_error" :src="mediaUrl(it.thumb_url)" :alt="it.name"
                 loading="lazy" @error="thumbFailed(it)" />
            <div v-else class="thumb-bad">缩略图不可用</div>
            <span v-if="it.violation" class="badge-l3">违规</span>
          </div>
          <div class="meta">
            <div class="name" :title="it.name">{{ it.name }}</div>
            <div class="muted">{{ it.ts }}</div>
            <div class="muted">
              摄像头 #{{ it.camera_id }} · 人 {{ it.persons }}
              <span v-if="it.in_zone"> · 在区 {{ it.in_zone }}</span>
              <span v-if="it.error" style="color:#dc2626"> · 读取失败</span>
            </div>
            <div v-if="it.violation" class="vio">{{ violationsText(it) }}</div>
          </div>
        </div>
      </div>
      <p class="muted" style="margin-top:10px">
        共 {{ items.length }} 张（最多显示最近 {{ LIMIT }} 张）· 点击图片查看原图与识别详情</p>
    </AsyncState>
  </div>

  <div v-if="detail" class="overlay" @click.self="detail = null">
    <div class="modal">
      <div class="row" style="justify-content:space-between; align-items:center">
        <h3 style="margin:0; word-break:break-all">{{ detail.name }}</h3>
        <button class="ghost" @click="detail = null">关闭</button>
      </div>
      <img :src="mediaUrl(detail.file_url)" class="full" :alt="detail.name" />
      <table style="margin-top:8px">
        <tr><th>图片时间</th><td>{{ detail.ts }}</td><th>识别时间</th><td>{{ detail.processed_ts }}</td></tr>
        <tr><th>分辨率</th><td>{{ detail.width }} × {{ detail.height }}</td>
            <th>识别结果</th><td>人员 {{ detail.persons }} · 在区 {{ detail.in_zone }}</td></tr>
        <tr><th>命中区域</th><td colspan="3">{{ (detail.zones || []).join('、') || '—' }}</td></tr>
        <tr><th>实验服证据</th><td colspan="3">
          未穿 {{ detail.ppe_no }} · 已穿 {{ detail.ppe_wearing }}</td></tr>
        <tr><th>违规</th><td colspan="3" :style="detail.violation ? 'color:var(--l3)' : ''">
          {{ violationsText(detail) || '无' }}</td></tr>
        <tr><th>关联事件</th><td colspan="3">
          <span v-if="detail.event_id">#{{ detail.event_id }}（可在「事件查询」页处置）</span>
          <span v-else class="muted">未触发事件（无匹配规则或处于冷却期）</span></td></tr>
        <tr v-if="detail.error"><th>错误</th><td colspan="3" style="color:#dc2626">{{ detail.error }}</td></tr>
        <tr><th>源文件</th><td colspan="3" style="word-break:break-all">{{ detail.path }}</td></tr>
      </table>
      <div class="row" style="margin-top:10px">
        <a class="link-btn" :href="mediaUrl(detail.file_url)" target="_blank" rel="noopener">查看原图</a>
        <a class="link-btn" :href="mediaUrl(detail.file_url)" :download="detail.name">下载原图</a>
        <span class="muted">关联事件 #{{ detail.event_id || '—' }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.panel { overflow-x: auto; }
.cards { flex-wrap: wrap; margin-bottom: 12px; }
.card { min-width: 140px; }
.grid {
  display: grid; gap: 12px; margin-top: 12px;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
}
.shot {
  border: 1px solid var(--divider); border-radius: 8px; overflow: hidden;
  background: var(--card-bg, transparent); cursor: zoom-in;
}
.shot:hover { border-color: var(--accent, #2563eb); }
.thumb-wrap { position: relative; aspect-ratio: 16 / 9; background: rgba(0, 0, 0, .06); }
.thumb-wrap img { width: 100%; height: 100%; object-fit: cover; display: block; }
.thumb-bad { display: flex; align-items: center; justify-content: center; height: 100%; font-size: 12px; color: #6b7280; }
.badge-l3 {
  position: absolute; top: 6px; right: 6px; background: #dc2626; color: #fff;
  font-size: 11px; padding: 1px 6px; border-radius: 10px; font-weight: 600;
}
.meta { padding: 8px 10px; font-size: 12px; }
.meta .name { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.vio { color: #dc2626; margin-top: 2px; }
.overlay {
  position: fixed; inset: 0; background: rgba(0, 0, 0, .85); z-index: 50;
  display: flex; align-items: flex-start; justify-content: center; overflow: auto; padding: 24px 12px;
}
.modal {
  background: var(--bg, #fff); color: var(--text, #111); border-radius: 10px;
  width: min(94vw, 1000px); padding: 16px; cursor: default;
}
.modal .full { width: 100%; max-height: 66vh; object-fit: contain; margin-top: 10px; background: #111; display: block; }
.link-btn {
  display: inline-block; padding: 7px 12px; border: 1px solid var(--divider);
  border-radius: 6px; text-decoration: none; color: inherit; font-size: 13px;
}
.link-btn:hover { border-color: var(--accent, #2563eb); }
</style>
