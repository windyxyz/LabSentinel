<script setup>
defineProps({
  samples: { type: Array, default: () => [] },
  names: { type: Object, default: () => ({}) },
})

const stages = [
  ['analysis_frame_age_ms', '帧龄'],
  ['read_wait_ms', '读取等待'],
  ['capture_queue_ms', '采集排队'],
  ['jpeg_ms', '预览编码'],
  ['detection_ms', '人员检测'],
  ['rules_with_ppe_ms', '规则与防护'],
  ['ort_run_ms', '模型推理'],
  ['directml_lock_wait_ms', 'GPU 等待'],
  ['analysis_after_read_ms', '分析合计'],
  ['event_work_after_read_ms', '事件处理'],
]

function format(value) {
  return Number.isFinite(value) ? value.toFixed(1) : '—'
}
</script>

<template>
  <section class="latency" aria-label="分析耗时">
    <div v-if="samples.length" class="latency-scroll">
      <table>
        <caption>各来源分析耗时 P95（毫秒）</caption>
        <thead><tr><th scope="col">来源</th><th v-for="[key, name] in stages" :key="key" scope="col">{{ name }}</th></tr></thead>
        <tbody>
          <tr v-for="sample in samples" :key="sample.camera_id">
            <th scope="row">{{ names[sample.camera_id] || '#' + sample.camera_id }}</th>
            <td v-for="[key] in stages" :key="key">{{ format(sample.latency?.[key]?.p95) }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <span v-else class="muted">来源开始分析后显示耗时</span>
  </section>
</template>

<style scoped>
.latency { flex-basis: 100%; font-size: 13px; min-width: 0; }
.latency-scroll { overflow-x: auto; }
table { width: 100%; border-collapse: collapse; white-space: nowrap; font-variant-numeric: tabular-nums; }
caption { text-align: left; margin: 4px 0; }
th, td { padding: 4px 8px; border-bottom: 1px solid var(--divider); text-align: right; }
th:first-child { text-align: left; }
</style>
