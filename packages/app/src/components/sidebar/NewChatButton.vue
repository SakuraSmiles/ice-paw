<script setup lang="ts">
// 新建会话按钮
//
// 职责：
//   - 简单按钮：「+ 新建会话」
//   - 点击 emit 'create'（外层接住后调 conversationsStore.create）
//
// emits:
//   - create  点击按钮时触发

const emit = defineEmits<{
  create: [];
}>();
</script>

<template>
  <button class="new-chat-btn" type="button" @click="emit('create')">+ 新建会话</button>
</template>

<style scoped>
.new-chat-btn {
  display: block;
  width: 100%;
  padding: 10px 14px;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  text-align: center;
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: background-color var(--ip-duration-immediate) var(--ip-ease-out),
    border-color var(--ip-duration-immediate) var(--ip-ease-out),
    color var(--ip-duration-immediate) var(--ip-ease-out);
}

.new-chat-btn:hover {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
  color: var(--ip-color-text-link);
}

.new-chat-btn:active {
  background: var(--ip-color-bg-tertiary);
}
</style>