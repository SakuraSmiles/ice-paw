<script setup lang="ts">
// 通用确认删除弹窗
//
// 职责：
//   - 显示确认信息，等待用户「确认」或「取消」
//   - 支持 Esc 关闭、点击遮罩关闭
//   - Agent 删除 / 会话删除等场景复用
//
// props:
//   - open:   是否显示
//   - title:  弹窗标题（如「确认删除」）
//   - message:提示正文（如「将删除该 Agent 及其所有会话，此操作不可撤销」）
//
// emits:
//   - confirm: 点击确认按钮时触发
//   - cancel:  点击取消 / Esc / 遮罩时触发

defineProps<{
  open: boolean;
  title: string;
  message: string;
}>();

const emit = defineEmits<{
  confirm: [];
  cancel: [];
}>();

/** Esc 关闭 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function handleKeydown(e: any): void {
  if (e.key === "Escape") {
    emit("cancel");
  }
}

/** 点击遮罩关闭 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
function handleOverlayClick(e: any): void {
  if (e.target.classList.contains("dialog-overlay")) {
    emit("cancel");
  }
}
</script>

<template>
  <Teleport to="body">
    <Transition name="dialog">
      <div
        v-if="open"
        class="dialog-overlay"
        @keydown="handleKeydown"
        @click="handleOverlayClick"
      >
        <div class="dialog-box" role="dialog" aria-modal="true">
          <h3 class="dialog-title">{{ title }}</h3>
          <p class="dialog-message">{{ message }}</p>
          <div class="dialog-actions">
            <button class="btn btn-cancel" @click="emit('cancel')">取消</button>
            <button class="btn btn-confirm" @click="emit('confirm')">确认</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.dialog-overlay {
  position: fixed;
  inset: 0;
  z-index: 10000;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.4);
  /* 让 overlay 接收键盘事件 */
  outline: none;
}

.dialog-box {
  width: 400px;
  max-width: calc(100vw - 32px);
  padding: 24px;
  background: var(--dialog-bg, #ffffff);
  border: 1px solid var(--dialog-border, #e0e0e0);
  border-radius: 10px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.12);
}

.dialog-title {
  margin: 0 0 12px;
  font-size: 17px;
  font-weight: 600;
  color: var(--text-primary, #1a1a1a);
}

.dialog-message {
  margin: 0 0 20px;
  font-size: 14px;
  line-height: 1.5;
  color: var(--text-secondary, #555);
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}

.btn {
  padding: 8px 18px;
  font-size: 14px;
  border-radius: 6px;
  border: 1px solid transparent;
  cursor: pointer;
  transition: background 100ms ease;
}

.btn-cancel {
  background: var(--btn-cancel-bg, #f0f0f0);
  color: var(--text-secondary, #555);
  border-color: var(--btn-cancel-border, #d0d0d0);
}
.btn-cancel:hover {
  background: var(--btn-cancel-bg-hover, #e0e0e0);
}

.btn-confirm {
  background: var(--danger-bg, #d93025);
  color: #fff;
}
.btn-confirm:hover {
  background: var(--danger-bg-hover, #b52a1f);
}

/* 动画 */
.dialog-enter-from {
  opacity: 0;
}
.dialog-enter-active {
  transition: opacity 150ms ease;
}
.dialog-leave-to {
  opacity: 0;
}
.dialog-leave-active {
  transition: opacity 150ms ease;
}

/* 暗色模式 */
@media (prefers-color-scheme: dark) {
  .dialog-box {
    --dialog-bg: #2a2a3a;
    --dialog-border: #3a3a4a;
  }
  .dialog-title {
    --text-primary: #f0f0f0;
  }
  .dialog-message {
    --text-secondary: #bbb;
  }
  .btn-cancel {
    --btn-cancel-bg: #3a3a4a;
    --btn-cancel-border: #4a4a5a;
    --btn-cancel-bg-hover: #4a4a5a;
    --text-secondary: #ccc;
  }
  .btn-confirm {
    --danger-bg: #c0392b;
  }
  .btn-confirm:hover {
    --danger-bg-hover: #a93226;
  }
}
</style>