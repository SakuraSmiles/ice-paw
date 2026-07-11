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
  font-size: 13px;
  font-weight: 500;
  text-align: center;
  color: var(--text-primary, #1a1a1a);
  background: var(--btn-bg, #ffffff);
  border: 1px solid var(--btn-border, #d0d0d0);
  border-radius: 6px;
  cursor: pointer;
  transition: background 80ms ease, border-color 80ms ease, color 80ms ease;
}

.new-chat-btn:hover {
  background: var(--btn-bg-hover, #f0f0f0);
  border-color: var(--btn-border-hover, #c0c0c0);
  color: var(--accent-fg, #1a73e8);
}

.new-chat-btn:active {
  background: var(--btn-bg-active, #ebebeb);
}

@media (prefers-color-scheme: dark) {
  .new-chat-btn {
    --text-primary: #f0f0f0;
    --btn-bg: #2a2a3a;
    --btn-border: #3a3a4a;
  }
  .new-chat-btn:hover {
    --btn-bg-hover: #353548;
    --btn-border-hover: #4a4a5a;
    --accent-fg: #6ba9e8;
  }
  .new-chat-btn:active {
    --btn-bg-active: #2a2a3a;
  }
}
</style>