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
  background: var(--ip-color-bg-overlay);
  /* 让 overlay 接收键盘事件 */
  outline: none;
}

.dialog-box {
  width: var(--ip-modal-w-sm);
  max-width: calc(100vw - 32px);
  padding: var(--ip-spacing-6);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-xl);
  box-shadow: var(--ip-shadow-lg);
}

.dialog-title {
  margin: 0 0 var(--ip-spacing-3);
  font-size: var(--ip-text-body-lg-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.dialog-message {
  margin: 0 0 var(--ip-spacing-5);
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-gray-700);
}

.dialog-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}

.btn {
  padding: var(--ip-btn-py-md) 18px;
  font-size: var(--ip-btn-fs-md);
  border-radius: var(--ip-btn-radius);
  border: 1px solid transparent;
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.btn-cancel {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-gray-700);
  border-color: var(--ip-color-border-default);
}
.btn-cancel:hover {
  background: var(--ip-gray-200);
}

.btn-confirm {
  background: var(--ip-danger-base);
  color: var(--ip-color-text-on-danger);
}
.btn-confirm:hover {
  background: var(--ip-danger-hover);
}

/* 动画 */
.dialog-enter-from,
.dialog-leave-to {
  opacity: 0;
}
.dialog-enter-active,
.dialog-leave-active {
  transition: opacity var(--ip-duration-base) var(--ip-ease-out);
}
</style>