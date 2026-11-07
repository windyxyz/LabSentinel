<script setup>
/** Q-08 / T-406：实验服模型按需下载（许可确认 + SHA256 校验）。 */
import { onMounted, ref } from 'vue'
import { api } from '../api'

const status = ref(null)
const busy = ref(false)
const msg = ref('')
const confirmed = ref(false)

async function load() {
  try {
    status.value = await api.ppeStatus()
    msg.value = ''
  } catch (e) {
    msg.value = String(e.message || e)
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
    status.value = await api.ppeDownload(true)
    msg.value = status.value.installed ? '下载完成并已启用实验服检测' : '下载未完成'
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
          <span v-if="status.installed && !status.active" class="muted">（文件已就绪，重启服务后生效）</span>
        </p>
        <p class="muted" style="white-space:pre-wrap">{{ status.license_notice }}</p>
        <p class="muted">
          校验 SHA256：<code>{{ status.sha256 }}</code><br />
          大小：{{ (status.size_bytes / 1048576).toFixed(1) }} MB；来源见 assets.lock.json
        </p>
        <template v-if="!status.installed">
          <label style="display:block;margin:10px 0">
            <input type="checkbox" v-model="confirmed" />
            我已阅读并接受上述数据集许可说明与用途限制
          </label>
          <button :disabled="busy || !confirmed" @click="download">
            {{ busy ? '下载中…' : '确认许可并下载模型' }}
          </button>
        </template>
        <p v-else class="st-ok">模型已安装且哈希校验通过。</p>
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
