<!-- LS-052：可视化周时段排班选择器。
     v-model 兼容现有后端契约：days_of_week（"1,2,3"）+ time_start/time_end（"HH:MM"）。 -->
<template>
  <div class="schedule-picker">
    <div class="row" style="align-items:center;flex-wrap:wrap;gap:6px">
      <span class="muted">快捷模板：</span>
      <button v-for="p in presets" :key="p.id" type="button" class="ghost"
              :class="{ active: isPreset(p) }" @click="applyPreset(p)">{{ p.label }}</button>
    </div>
    <div class="row" style="align-items:center;flex-wrap:wrap;gap:6px;margin-top:6px">
      <span class="muted">生效日：</span>
      <label v-for="(w, i) in weekdayLabels" :key="w" class="muted" style="margin-right:4px">
        <input type="checkbox" :checked="days.includes(i + 1)" @change="toggleDay(i + 1)" />
        周{{ w }}</label>
    </div>
    <div class="row" style="align-items:center;gap:6px;margin-top:6px">
      <span class="muted">时段：</span>
      <input type="time" :value="start" aria-label="开始时间" @input="setStart($event.target.value)" />
      <span class="muted">—</span>
      <input type="time" :value="end" aria-label="结束时间" @input="setEnd($event.target.value)" />
      <span v-if="overnight" class="muted" style="font-size:12px">跨午夜</span>
      <span class="muted" style="font-size:12px">{{ summary }}</span>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import {
  SCHEDULE_PRESETS, WEEKDAY_LABELS, describeSchedule,
  formatDays, isOvernight, normalizeWindow, parseDays,
} from '../schedule'

const props = defineProps({
  daysOfWeek: { type: String, default: '' },
  timeStart: { type: String, default: '00:00' },
  timeEnd: { type: String, default: '23:59' },
})
const emit = defineEmits(['update:daysOfWeek', 'update:timeStart', 'update:timeEnd'])

const presets = SCHEDULE_PRESETS
const weekdayLabels = WEEKDAY_LABELS
const days = computed(() => parseDays(props.daysOfWeek))
const start = computed(() => normalizeWindow(props.timeStart, props.timeEnd).start)
const end = computed(() => normalizeWindow(props.timeStart, props.timeEnd).end)
const overnight = computed(() => isOvernight(props.timeStart, props.timeEnd))
const summary = computed(() => describeSchedule(props.daysOfWeek, props.timeStart, props.timeEnd))

function isPreset(p) {
  return formatDays(parseDays(p.days)) === formatDays(days.value)
    && p.start === start.value && p.end === end.value
}

function applyPreset(p) {
  emit('update:daysOfWeek', p.days)
  emit('update:timeStart', p.start)
  emit('update:timeEnd', p.end)
}

function toggleDay(n) {
  const set = new Set(days.value)
  if (set.has(n)) set.delete(n)
  else set.add(n)
  emit('update:daysOfWeek', formatDays([...set]))
}

function setStart(v) {
  emit('update:timeStart', normalizeWindow(v, end.value).start)
}
function setEnd(v) {
  emit('update:timeEnd', normalizeWindow(start.value, v).end)
}
</script>

<style scoped>
.schedule-picker { margin-top: 4px; }
.schedule-picker .ghost.active { border-color: var(--accent); color: var(--accent); }
.schedule-picker input[type='time'] { width: 110px; }
</style>
