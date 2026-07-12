<script setup lang="ts">
// 空状态提示组件
//
// 职责：
//   - Agent 列表为空时显示欢迎引导
//   - 提供「创建 Agent」入口按钮
//
// emits:
//   - create: 点击创建按钮时触发

const emit = defineEmits<{
  create: [];
}>();
</script>

<template>
  <div class="empty-hint">
    <h2 class="empty-title">欢迎使用 IcePaw</h2>
    <p class="empty-desc">当前还没有 Agent，创建你的第一个 Agent 开始对话。</p>
    <button class="btn-create" @click="emit('create')">创建 Agent</button>
  </div>
</template>

<style scoped>
.empty-hint {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 48px var(--ip-spacing-6);
  text-align: center;
}

.empty-title {
  margin: 0 0 var(--ip-spacing-3);
  font-size: var(--ip-text-h2-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.empty-desc {
  margin: 0 0 var(--ip-spacing-6);
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-tertiary);
  max-width: 320px;
}

.btn-create {
  padding: var(--ip-btn-py-md) var(--ip-btn-px-lg);
  font-size: var(--ip-btn-fs-md);
  font-weight: var(--ip-font-weight-medium);
  border: none;
  border-radius: var(--ip-btn-radius);
  background: var(--ip-primary-500);
  color: var(--ip-color-text-on-primary);
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-create:hover {
  background: var(--ip-primary-600);
}
</style>