<script setup>
import { computed, ref, watch } from 'vue'
import { mediaUrl } from '../api'

const props = defineProps({ eventId: { type: Number, required: true }, autoplay: Boolean })
const unavailable = ref(false)
const source = computed(() => mediaUrl(`/api/events/${props.eventId}/video`))
watch(() => props.eventId, () => { unavailable.value = false })
</script>

<template>
  <div class="event-video">
    <video :src="source" controls :autoplay="autoplay" preload="metadata"
           aria-label="事件视频回放" @error="unavailable = true" />
    <p v-if="unavailable" class="muted" role="status">
      播放不可用。可查看事件截图或 <a :href="source" :download="`event-${eventId}.mp4`">下载原录像</a>。
    </p>
  </div>
</template>

<style scoped>
video { display: block; width: 100%; max-height: 92vh; border-radius: var(--overlay-radius); background: #000; }
p { margin: 6px 0; }
</style>
