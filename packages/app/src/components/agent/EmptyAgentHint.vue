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
  padding: 48px 24px;
  text-align: center;
}

.empty-title {
  margin: 0 0 12px;
  font-size: 22px;
  font-weight: 600;
  color: var(--text-primary, #1a1a1a);
}

.empty-desc {
  margin: 0 0 24px;
  font-size: 14px;
  line-height: 1.6;
  color: var(--text-secondary, #888);
  max-width: 320px;
}

.btn-create {
  padding: 10px 24px;
  font-size: 14px;
  font-weight: 500;
  border: none;
  border-radius: 6px;
  background: var(--accent-bg, #1a73e8);
  color: #fff;
  cursor: pointer;
  transition: background 100ms ease;
}
.btn-create:hover {
  background: var(--accent-bg-hover, #1557b0);
}

/* 暗色模式 */
@media (prefers-color-scheme: dark) {
  .empty-title {
    --text-primary: #f0f0f0;
  }
  .empty-desc {
    --text-secondary: #888;
  }
  .btn-create {
    --accent-bg: #4a90e2;
  }
  .btn-create:hover {
    --accent-bg-hover: #357abd;
  }
}
</style>