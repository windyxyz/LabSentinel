<script setup>
import { onMounted, ref } from 'vue'
import { api } from '../api'

const settings = ref(null)
const busy = ref(false)
const error = ref('')
const source = 'https://huggingface.co/onnx-community/siglip2-base-patch16-224-ONNX'

async function load() {
  busy.value = true
  error.value = ''
  try { settings.value = await api.semanticStatus() }
  catch (err) { error.value = err.message }
  finally { busy.value = false }
}
async function toggle() {
  const enabled = !settings.value?.enabled
  busy.value = true
  error.value = ''
  try {
    await api.semanticConfigure(enabled)
    settings.value = await api.semanticStatus()
  } catch (err) { error.value = err.message }
  finally { busy.value = false }
}
onMounted(load)
</script>

<template>
  <h3>事件离线搜图</h3>
  <p class="muted">启用后在本机 CPU 逐张索引事件截图。事件页可用中英文描述搜图，也可从已有事件找相似画面。</p>
  <div class="row">
    <button :disabled="busy || !settings || (!settings.available && !settings.enabled)" @click="toggle">
      {{ settings?.enabled ? '关闭离线搜图' : '开启离线搜图' }}
    </button>
    <button class="ghost" :disabled="busy" @click="load">重新检查文件</button>
  </div>
  <p v-if="settings && !settings.available" class="muted">
    将 <code>tokenizer.json</code>、<code>text_model_quantized.onnx</code> 和
    <code>vision_model_quantized.onnx</code> 放到数据目录下的 <code>{{ settings.directory }}</code>。
    <a :href="source" target="_blank" rel="noopener noreferrer">查看模型文件</a>
  </p>
  <p v-if="error" role="alert">{{ error }}</p>
</template>
