<script setup>
import { ref } from 'vue'
import { api, mediaUrl, TYPE_TEXT } from '../api'

const emit = defineEmits(['open-event'])
const open = ref(false)
const q = ref('')
const busy = ref(false)
const results = ref(null)
const error = ref('')

async function search(params) {
  if (busy.value) return
  open.value = true
  busy.value = true
  results.value = null
  error.value = ''
  try {
    const response = await api.semanticSearch(params)
    results.value = Array.isArray(response?.results)
      ? response.results.filter(item => Number.isInteger(item?.event?.id)) : []
  } catch (err) {
    error.value = err.message
  } finally {
    busy.value = false
  }
}
function similar(id) { return search({ similar_to: id }) }
defineExpose({ similar })
</script>

<template>
  <details :open="open" @toggle="open = $event.target.open">
    <summary>离线搜图</summary>
    <form class="row" @submit.prevent="search({ q: q.trim() })">
      <input v-model="q" maxlength="256" aria-label="描述要找的画面" placeholder="例如 穿白大褂的人" />
      <button :disabled="busy || !q.trim()">搜索</button>
    </form>
    <p v-if="busy" role="status">正在匹配截图…</p>
    <p v-if="error" role="alert">{{ error }}</p>
    <p v-else-if="results && !results.length" class="muted">暂无可访问的已索引截图。</p>
    <div v-else-if="results" class="explore-results">
      <button v-for="item in results" :key="item.event.id" type="button" class="ghost"
              @click="emit('open-event', item.event)">
        <img loading="lazy" :src="mediaUrl(`/api/events/${item.event.id}/thumb`)" alt="" />
        <span>{{ TYPE_TEXT[item.event.type] || item.event.type }} · #{{ item.event.id }}</span>
        <small>{{ item.event.ts }} · 机位 {{ item.event.camera_id }}</small>
      </button>
    </div>
  </details>
</template>

<style scoped>
details { margin-block: 10px; }
form { margin-top: 8px; }
.explore-results { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 8px; margin-top: 8px; }
.explore-results button { display: grid; gap: 4px; text-align: left; min-width: 0; }
.explore-results img { width: 100%; aspect-ratio: 16 / 9; object-fit: cover; border-radius: var(--overlay-radius); }
</style>
