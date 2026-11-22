<script setup>
/** 实验服（PPE）模型：随包提供 + 启停开关 + 上传替换（0.4.0）。 */
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'

const status = ref(null)
const busy = ref(false)
const uploading = ref(false)
const msg = ref('')
const confirmed = ref(false)
const fileEl = ref(null)

const SOURCE_TEXT = {
  bundled: '随安装包提供',
  custom: '自定义上传',
  env: '环境变量指定',
  none: '未找到模型',
}

async function load() {
  try {
    status.value = await api.ppeStatus()
  } catch (e) {
    msg.value = String(e.message || e)
  }
}

async function toggle(ev) {
  const enabled = ev.target.checked
  busy.value = true
  msg.value = ''
  try {
    const r = await api.ppeConfig({ enabled })
    await load()
    msg.value = enabled
      ? `已开启实验服检测${r.restarted ? `（${r.restarted} 路相机已重启生效）` : ''}`
      : `已关闭实验服检测（仅人员检测）${r.restarted ? `，${r.restarted} 路相机已重启` : ''}`
  } catch (e) {
    msg.value = String(e.message || e)
    await load()
  } finally {
    busy.value = false
  }
}

// LS-055：模型 A/B 对比评测
const benchBusy = ref(false)
const bench = ref(null)
const benchPath = ref('')
const benchSides = computed(() => [
  { title: '当前模型', data: bench.value?.current },
  { title: '候选模型', data: bench.value?.candidate },
])
async function runBenchmark() {
  const path = prompt('候选模型 .onnx 的完整路径：')
  if (!path || !path.trim()) return
  benchBusy.value = true
  bench.value = null
  msg.value = ''
  try {
    bench.value = await api.ppeBenchmark(path.trim())
    benchPath.value = path.trim()
    msg.value = bench.value?.ok ? '对比评测完成' : '评测中发现模型错误'
  } catch (e) { msg.value = String(e.message || e) }
  benchBusy.value = false
}
async function activateCandidate() {
  benchBusy.value = true
  try {
    const result = await api.ppeBenchmark(benchPath.value, true)
    if (!result?.candidate || result.candidate.error) throw new Error(result?.candidate?.error || '候选模型不可用')
    await load()
    bench.value = null
    msg.value = '已启用候选模型'
  } catch (err) { msg.value = err.message }
  finally { benchBusy.value = false }
}

async function saveThreshold(key, raw) {
  const val = Number(raw)
  if (!Number.isFinite(val) || val < 0.05 || val > 0.95) return
  busy.value = true
  msg.value = ''
  try {
    const r = await api.ppeConfig({ [key]: val })
    await load()
    msg.value = `已保存${key === 'violation_conf' ? '违规' : '穿戴'}置信度${r.restarted ? `（${r.restarted} 路相机已重启）` : ''}`
  } catch (e) {
    msg.value = String(e.message || e)
    await load()
  } finally {
    busy.value = false
  }
}

function pickFile() {
  fileEl.value?.click()
}

async function onFile(ev) {
  const f = ev.target.files?.[0]
  ev.target.value = ''
  if (!f) return
  if (!confirm(`上传并启用「${f.name}」（${(f.size / 1048576).toFixed(1)} MB）？\n将替换当前实验服模型，可在之后点「恢复随包模型」。`)) return
  uploading.value = true
  msg.value = '上传中…'
  try {
    const r = await api.ppeUpload(f)
    await load()
    msg.value = `已上传并启用${r.restarted ? `（${r.restarted} 路相机已重启）` : ''}`
  } catch (e) {
    msg.value = `上传失败: ${e.message || e}`
  } finally {
    uploading.value = false
  }
}

async function reset() {
  if (!confirm('恢复为安装包自带的实验服模型？')) return
  busy.value = true
  msg.value = ''
  try {
    const r = await api.ppeReset()
    await load()
    msg.value = `已恢复随包模型${r.restarted ? `（${r.restarted} 路相机已重启）` : ''}`
  } catch (e) {
    msg.value = String(e.message || e)
  } finally {
    busy.value = false
  }
}

async function download() {
  if (!confirmed.value) {
    msg.value = '请先勾选许可确认'
    return
  }
  busy.value = true
  msg.value = '下载中（含 SHA256 校验）…'
  try {
    const r = await api.ppeDownload(true)
    status.value = { ...(status.value || {}), ...(r || {}) }
    msg.value = status.value?.installed ? '下载完成并已启用实验服检测' : '下载未完成'
  } catch (e) {
    msg.value = `下载失败: ${e.message || e}`
  } finally {
    busy.value = false
  }
}

onMounted(load)
</script>

<template>
  <div class="models-page">
    <h2>模型管理</h2>
    <div class="card">
      <h3>实验服（PPE）检测模型</h3>
      <p v-if="!status">加载中…</p>
      <template v-else>
        <p>
          当前模式：
          <b :class="status.mode === 'ppe' ? 'st-ok' : 'st-idle'">
            {{ status.mode === 'ppe' ? '人员 + 实验服检测' : '仅人员检测' }}
          </b>
          <span class="muted">（模型来源：{{ SOURCE_TEXT[status.source] || status.source || '—' }}）</span>
          <span v-if="status.installed && !status.enabled" class="muted">
            ——已关闭，打开下方开关即启用
          </span>
        </p>
        <label v-if="status.installed" class="ppe-switch" style="display:flex;align-items:center;gap:10px;margin:10px 0">
          <input type="checkbox" :checked="status.enabled" :disabled="busy"
                 @change="toggle" />
          <span>启用实验服检测</span>
        </label>
        <p class="muted" style="white-space:pre-wrap">{{ status.license_notice }}</p>
        <p v-if="status.warning" class="lv2" style="font-size:12.5px">{{ status.warning }}</p>
        <!-- LS-050：PPE 置信度阈值（改完即存，运行中相机软重启生效） -->
        <div class="ppe-thresholds" style="margin:10px 0">
          <label class="muted">
            违规置信度 <b>{{ (status.violation_conf ?? 0.3).toFixed(2) }}</b>
            <input type="range" min="0.05" max="0.95" step="0.05"
                   :value="status.violation_conf ?? 0.3" :disabled="busy"
                   @change="saveThreshold('violation_conf', $event.target.value)" />
          </label>
          <label class="muted" style="margin-left:16px">
            穿戴置信度 <b>{{ (status.wearing_conf ?? 0.2).toFixed(2) }}</b>
            <input type="range" min="0.05" max="0.95" step="0.05"
                   :value="status.wearing_conf ?? 0.2" :disabled="busy"
                   @change="saveThreshold('wearing_conf', $event.target.value)" />
          </label>
          <span class="muted" style="display:block;font-size:12px;margin-top:4px">
            灰度/远景监控图若检出偏低，可适当降低违规置信度；阈值越低越敏感但误报增多。</span>
        </div>
        <p v-if="status.classes?.length" class="muted" style="font-size:12px">
          模型类别（{{ status.classes.length }}）：{{ status.classes.join(' / ') }}</p>
        <p class="muted">
          校验 SHA256：<code>{{ status.sha256 || '—' }}</code><br />
          大小：<template v-if="status.size_bytes > 0">{{ (status.size_bytes / 1048576).toFixed(1) }} MB</template><template v-else>—</template>
          <template v-if="!status.bundled">；来源见 assets.lock.json</template>
        </p>
        <div class="row" style="margin-top:10px;align-items:center;flex-wrap:wrap">
          <button class="ghost" :disabled="uploading" @click="pickFile">
            {{ uploading ? '上传中…' : '上传替换模型（.onnx）' }}</button>
          <button class="ghost" :disabled="busy || status.source !== 'custom'" @click="reset">
            恢复随包模型</button>
          <input ref="fileEl" type="file" accept=".onnx" style="display:none" @change="onFile" />
          <button class="ghost" :disabled="benchBusy" @click="runBenchmark">
            {{ benchBusy ? '评测中…' : '对比评测（A/B）' }}</button>
          <span class="muted" style="font-size:12px">
            上传后立即启用；替换文件保存在数据目录 models\ppe-custom.onnx，卸载不删除用户数据。</span>
        </div>
        <dialog v-if="bench" open class="card" aria-label="模型对比评测">
          <p>本次评测 {{ bench.images }} 张图片</p>
          <section v-for="{ title, data } in benchSides" :key="title">
            <b>{{ title }}</b>
            <p v-if="data?.error" class="lv3">{{ data.error }}</p>
            <template v-else-if="data">
              <p>{{ data.avg_ms?.toFixed?.(1) ?? '—' }} ms/帧 · 检出 {{ data.detections }}</p>
              <p v-if="Array.isArray(data.input)">{{ data.format }} · {{ data.input.join(' × ') }}</p>
              <div v-for="item in (Array.isArray(data.classes) ? data.classes : [])" :key="item.class">{{ item.class }} · {{ item.count }}</div>
              <details v-if="Array.isArray(data.per_image)">
                <summary>逐图检出</summary>
                <div v-for="row in data.per_image" :key="row.index">
                  第 {{ row.index }} 张 · {{ row.ms?.toFixed?.(1) }} ms · {{ row.detections }} 项
                </div>
              </details>
            </template>
          </section>
          <button v-if="bench.candidate && !bench.candidate.error" :disabled="benchBusy" @click="activateCandidate">启用候选模型</button>
          <button class="ghost" @click="bench = null">关闭</button>
        </dialog>
        <template v-if="!status.installed">
          <label style="display:block;margin:10px 0">
            <input type="checkbox" v-model="confirmed" />
            我已阅读并接受上述数据集许可说明与用途限制
          </label>
          <button :disabled="busy || !confirmed" @click="download">
            {{ busy ? '下载中…' : '确认许可并下载模型' }}
          </button>
        </template>
      </template>
      <p v-if="msg" class="muted" style="margin-top:8px">{{ msg }}</p>
    </div>
    <div class="card">
      <h3>人体检测模型</h3>
      <p class="muted">
        YOLO11n（Ultralytics，AGPL-3.0）随包分发；来源与许可见 NOTICE 与
        models/model.json 角色声明。无需下载。
      </p>
    </div>
  </div>
</template>
