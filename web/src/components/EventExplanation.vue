<script setup>
import { computed, ref } from 'vue'
import { api } from '../api'

const props = defineProps({ eventId: { type: Number, required: true } })
const value = ref(null)
const busy = ref(false)
const message = ref('')
const labels = {
  bottom_center: '脚下', body_center: '身体中心', upper_body: '上身', four_point_any: '四点任一入区',
  covered: '已穿戴', wearing: '已穿戴', no_ppe: '未穿戴', unknown: '看不清',
  likely_true: '疑似真实', likely_false: '疑似误报', uncertain: '不确定',
  recovered: '确认恢复', left_zone: '确认离场', interrupted: '观测中断',
  person_review_window: '同一人员在复核间隔内已有送审记录',
  target_not_isolated: '人物重叠，当前画面无法单独复核',
  no_detection: '整图和可用裁剪中未检出明确证据', conflicting_labels: '穿戴类别证据冲突',
  overlapping_people: '人员重叠，无法唯一归属', model_error: '穿戴模型推理失败',
  confirmation_error: '末级复核不可用', confirmation_no_match: '末级复核未找到此人的有效证据',
  confirmation_not_positive: '末级复核的正向证据不足', classified: '已取得明确穿戴证据',
  local_geometry_rule: '由本地规则判断', budget_exhausted: '今日复核额度已用完',
  local_candidate: '本地画面提示', local_confirmed: '本地规则已确认', vision_confirmed: '图像复核已确认',
}
const percent = (value) => typeof value === 'number' && Number.isFinite(value) ? Math.round(value * 100) + '%' : '—'
const visual = computed(() => value.value?.visual)
const zone = computed(() => value.value?.zone)
const episode = computed(() => value.value?.episode)
const review = computed(() => value.value?.review)
const label = (key) => labels[key] || '—'

async function load() {
  if (busy.value) return
  busy.value = true
  message.value = ''
  try {
    const result = await api.eventExplanation(props.eventId)
    if (result?.id !== props.eventId) throw new Error('事件说明暂时不可用')
    value.value = result
  } catch (error) {
    message.value = error.message
  } finally {
    busy.value = false
  }
}
</script>

<template>
  <details class="explanation" @toggle="$event.target.open && !value && load()">
    <summary>检测与复核依据</summary>
    <p v-if="busy && !value" role="status">正在读取依据…</p>
    <p v-if="message" role="status">{{ message }}</p>
    <dl v-if="value">
      <template v-if="value.provenance?.settings">
        <dt>当时使用的配置</dt>
        <dd>版本 {{ value.provenance.settings.version }} · 推理首选 {{ value.provenance.settings.requested_backend }}</dd>
        <dd v-if="value.provenance.settings.person_model">人员模型 {{ value.provenance.settings.person_model.name }}</dd>
        <dd v-if="value.provenance.settings.ppe_model">穿戴模型 {{ value.provenance.settings.ppe_model.name }}</dd>
        <dd>配置摘要 <code :title="value.provenance.sha256">{{ value.provenance.sha256?.slice(0, 12) }}</code></dd>
      </template>
      <template v-if="visual">
        <dt>本地检测</dt>
        <dd v-if="visual.person_confidence != null">人员置信度 {{ percent(visual.person_confidence) }}</dd>
        <dd v-if="visual.violation_confidence != null">违规证据置信度 {{ percent(visual.violation_confidence) }}</dd>
        <dd v-if="visual.assignment_gap != null">服装归属重叠差值 {{ percent(visual.assignment_gap) }}</dd>
        <dd v-if="visual.coat != null">实验服 {{ label(visual.coat) }}</dd>
        <dd v-if="visual.coat_diagnostic?.result">{{ visual.coat_diagnostic.input === 'person_crop' ? '人员裁剪' : '整图' }} · {{ label(visual.coat_diagnostic.result) }}</dd>
        <dd v-if="Array.isArray(visual.coat_diagnostic?.confirmation_scores)">末级正证据 {{ percent(visual.coat_diagnostic.confirmation_scores[0]) }} · 反证 {{ percent(visual.coat_diagnostic.confirmation_scores[1]) }}</dd>
        <dd v-if="visual.mask != null">口罩 {{ label(visual.mask) }}</dd>
        <dd v-if="visual.gloves != null">手套 {{ label(visual.gloves) }}</dd>
        <dd v-if="visual.goggles != null">护目镜 {{ label(visual.goggles) }}</dd>
        <dd v-if="visual.occluded">抓拍时发生遮挡</dd>
      </template>
      <template v-if="value.error_code">
        <dt>专项规则</dt>
        <dd>{{ label(value.confidence_stage) }}</dd>
        <dd v-if="value.suggestion_tts">{{ value.suggestion_tts }}</dd>
      </template>
      <template v-if="zone?.spatial">
        <dt>区域判定</dt>
        <dd>{{ zone.name }} · {{ label(zone.spatial.anchor_mode) }} · 边界宽容 {{ zone.spatial.margin_px }} 像素</dd>
      </template>
      <template v-if="episode">
        <dt>违规片段</dt>
        <dd>{{ episode.first_ts }} 至 {{ episode.last_ts }} · {{ Number(episode.duration_s || 0).toFixed(1) }} 秒 · {{ episode.hit_count }} 次</dd>
        <dd v-if="episode.context?.close_reason">{{ label(episode.context.close_reason) }} · {{ episode.context.closed_ts }}</dd>
      </template>
      <dt>AI 复核</dt>
      <dd v-if="review?.verdict">{{ label(review.verdict) }} · {{ percent(review.confidence) }}<template v-if="value.review_model"> · {{ value.review_model }}</template></dd>
      <dd v-else-if="review?.status === 'unreviewed'">已保留本地检测结果</dd>
      <dd v-else>待复核</dd>
      <dd v-if="typeof review?.reason === 'string'">{{ labels[review.reason] || review.reason }}</dd>
      <dd v-if="value.delivery_suppressed">复核后已抑制告警投递</dd>
    </dl>
    <button class="ghost" type="button" :disabled="busy" @click="load">刷新依据</button>
  </details>
</template>

<style scoped>
.explanation { border-top: 1px solid var(--border); padding-block: 8px; }
summary { cursor: pointer; font-weight: 600; }
dl { margin-block: 8px; }
dt { font-weight: 600; margin-top: 10px; }
dd { margin-inline-start: 0; color: var(--text-secondary); overflow-wrap: anywhere; }
</style>
