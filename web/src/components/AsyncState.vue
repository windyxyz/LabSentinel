<script setup>
// 任务66：四页统一的 加载/错误/空 三态占位组件（不引 UI 框架，样式全 scoped）。
// 用法：<AsyncState :loading :error :empty empty-text @retry>…内容 slot…</AsyncState>
// notice：轮询失败但保留旧数据时的顶部轻提示（不打断内容展示）。
defineProps({
  loading: Boolean,
  error: { type: String, default: '' },
  empty: Boolean,
  emptyText: { type: String, default: '暂无数据' },
  notice: { type: String, default: '' },
})
defineEmits(['retry'])
</script>

<template>
  <div v-if="notice" class="as-notice" role="status">{{ notice }}</div>
  <div v-if="loading" class="as-loading" role="status">加载中…</div>
  <div v-else-if="error" class="as-error" role="alert">
    <span class="as-error-msg">{{ error }}</span>
    <button class="ghost" @click="$emit('retry')">重试</button>
    <span v-if="/401|令牌/.test(error)" class="as-error-hint">（401：到「摄像头」页顶部填写管理令牌）</span>
  </div>
  <div v-else-if="empty" class="as-empty">{{ emptyText }}</div>
  <template v-else>
    <slot />
  </template>
</template>

<style scoped>
.as-loading { color: var(--text-secondary, #6b7280); padding: 14px 0; }
.as-error {
  display: flex; flex-wrap: wrap; align-items: center; gap: 8px;
  color: #dc2626; background: rgba(220, 38, 38, 0.08);
  border: 1px solid rgba(220, 38, 38, 0.35);
  border-radius: var(--overlay-radius, 8px); padding: 10px 12px; margin: 8px 0;
  font-size: 13px;
}
.as-error-msg { word-break: break-all; }
.as-error-hint { color: var(--text-secondary, #6b7280); }
.as-empty { color: var(--text-secondary, #6b7280); padding: 14px 0; }
.as-notice {
  color: var(--text-secondary, #6b7280); font-size: 12px;
  background: rgba(180, 83, 9, 0.08); border-radius: var(--overlay-radius, 8px);
  padding: 6px 10px; margin-bottom: 8px;
}
</style>
