<script setup>
// RA5-UX：目录选择器对话框——盘符/目录两级浏览，替代手输路径。
// 数据源 GET /api/fs/list?path=（后端 admin 门控）。
import { ref } from 'vue'

const props = defineProps({
  title: { type: String, default: '选择文件夹' },
})
const emit = defineEmits(['pick', 'close'])
const open = ref(false)
const cur = ref('')
const entries = ref([])
const drives = ref([])
const loading = ref(false)
const err = ref('')

async function load(path) {
  loading.value = true
  err.value = ''
  try {
    const op = window.fetch
    const r = await op(`/api/fs/list?path=${encodeURIComponent(path)}`, {
      headers: { 'X-Admin-Token': localStorage.getItem('token') || '' },
    })
    const j = await r.json()
    cur.value = j.path || path
    if (j.drives) drives.value = j.drives
    entries.value = j.entries || []
    open.value = true
  } catch (e) {
    err.value = String(e.message || e)
  } finally {
    loading.value = false
  }
}

function openAt(startPath) {
  load(startPath || '')
}
defineExpose({ openAt })

function enter(e) {
  if (e.is_dir) load(e.path)
}
function pickHere() {
  emit('pick', cur.value)
  open.value = false
}
</script>

<template>
  <div v-if="open" class="fsp-overlay" @click.self="open = false">
    <div class="fsp-box">
      <div class="fsp-head">
        <b>{{ title }}</b>
        <button class="ghost" @click="open = false">关闭</button>
      </div>
      <div class="fsp-path">
        <code>{{ cur || '（选择盘符）' }}</code>
      </div>
      <div v-if="loading" class="fsp-load">加载中…</div>
      <div v-else-if="err" class="fsp-load">{{ err }}</div>
      <div class="fsp-list">
        <div v-if="drives.length" class="fsp-sec">盘符</div>
        <div v-for="d in drives" :key="d.path" class="fsp-item dir" @dblclick="load(d.path)">
          <span class="fsp-ico">💾</span>{{ d.name }}
          <button class="ghost fsp-open" @click.stop="load(d.path)">打开</button>
        </div>
        <div v-if="cur" class="fsp-sec">当前目录子项</div>
        <div v-for="e in entries" :key="e.path" class="fsp-item" :class="{ dir: e.is_dir }"
             @dblclick="e.is_dir && load(e.path)">
          <span class="fsp-ico">{{ e.is_dir ? '📁' : '📄' }}</span>{{ e.name }}
          <button v-if="e.is_dir" class="ghost fsp-open" @click.stop="load(e.path)">打开</button>
        </div>
        <div v-if="cur && !entries.length" class="fsp-load">（空目录）</div>
      </div>
      <div class="fsp-foot">
        <span class="muted">双击进入子目录</span>
        <button class="primary" :disabled="!cur" @click="pickHere">选择此目录</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.fsp-overlay { position:fixed; inset:0; background:rgba(0,0,0,.45); z-index:1000;
  display:flex; align-items:center; justify-content:center; }
.fsp-box { background:var(--panel, #fff); border-radius:8px; width:520px; max-width:92vw;
  max-height:80vh; display:flex; flex-direction:column; padding:14px; }
.fsp-head { display:flex; justify-content:space-between; align-items:center; margin-bottom:8px; }
.fsp-path { background:var(--divider, #eee); border-radius:6px; padding:6px 8px; margin-bottom:8px;
  word-break:break-all; }
.fsp-list { flex:1; overflow-y:auto; max-height:46vh; border:1px solid var(--divider, #ddd);
  border-radius:6px; }
.fsp-sec { font-size:12px; color:var(--muted, #888); padding:6px 8px 2px; }
.fsp-item { padding:6px 10px; cursor:pointer; display:flex; align-items:center; gap:6px; }
.fsp-item:hover { background:var(--divider, #f0f0f0); }
.fsp-item.dir { font-weight:600; }
.fsp-ico { width:20px; text-align:center; }
.fsp-open { margin-left:auto; font-size:12px; }
.fsp-load { padding:12px; color:var(--muted, #888); }
.fsp-foot { display:flex; justify-content:space-between; align-items:center; margin-top:10px; }
.primary { background:var(--accent, #1b3366); color:#fff; border:none; border-radius:6px;
  padding:6px 14px; cursor:pointer; }
.ghost { background:none; border:1px solid var(--divider, #ccc); border-radius:6px;
  padding:4px 10px; cursor:pointer; }
.muted { color:var(--muted, #888); }
</style>
