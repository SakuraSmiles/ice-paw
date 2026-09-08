<!--
  EmbedSwitchOverlay — 语义检索切换/变更重建二次确认浮层（设置-通用 · 语义检索卡
  与设置-模型 · 被引用 profile 编辑共用）。面板形状一致、confirm 逻辑由调用方持有：
  切换 = test → save → rebuild；编辑 = update → test → rebuild。错误文案由调用方
  生成（两态语义不同：未切换 vs 已更新未重建），本组件只管展示与按钮态。
-->
<script setup lang="ts">
defineProps<{
  title: string;
  rows: { label: string; value: string }[];
  error?: string | null;
  rebuilding?: boolean;
}>();

defineEmits<{ cancel: []; confirm: [] }>();
</script>

<template>
  <div class="embed-switch-overlay" @click.self="$emit('cancel')">
    <div class="embed-switch-panel" @click.stop>
      <h3>{{ title }}</h3>
      <p v-for="(r, i) in rows" :key="i" class="embed-switch-row">{{ r.label }}：<b>{{ r.value }}</b></p>
      <p class="embed-switch-warn">模型变更后知识库向量将失效并自动重建（可能需几十秒）。</p>
      <div v-if="error" class="embed-switch-error">{{ error }}</div>
      <div class="embed-switch-actions">
        <button class="btn" :disabled="rebuilding" @click="$emit('cancel')">取消</button>
        <button class="btn-primary" :disabled="rebuilding" @click="$emit('confirm')">
          {{ rebuilding ? "重建中…" : "确认并重建" }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.embed-switch-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: var(--ip-z-dropdown);
}
.embed-switch-panel {
  width: 380px;
  max-width: 90vw;
  padding: 20px 22px;
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: 12px;
  box-shadow: var(--ip-shadow-lg);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}
.embed-switch-panel h3 {
  margin: 0 0 4px;
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.embed-switch-row {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
}
.embed-switch-warn {
  margin: 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.5;
}
.embed-switch-error {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-danger-text);
  line-height: 1.5;
}
.embed-switch-actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--ip-spacing-2);
  margin-top: 6px;
}
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: var(--ip-input-h-sm);
  padding: 0 12px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  white-space: nowrap;
}
.btn:disabled { opacity: 0.6; cursor: not-allowed; }
.btn-primary {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: var(--ip-input-h-sm);
  padding: 0 12px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: white;
  background-color: var(--ip-primary-600);
  border: none;
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  white-space: nowrap;
}
.btn-primary:disabled { opacity: 0.6; cursor: not-allowed; }
</style>
