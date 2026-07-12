<script setup lang="ts">
// 通用确认删除弹窗
//
// 职责：
//   - 显示确认信息，等待用户「确认」或「取消」
//   - 支持 Esc 关闭、点击遮罩关闭（通过 @ice-paw/ui 的 Modal 实现）
//   - Agent 删除 / 会话删除等场景复用
//
// props:
//   - open:   是否显示（v-model:open）
//   - title:  弹窗标题（如「确认删除」）
//   - message:提示正文（如「将删除该 Agent 及其所有会话，此操作不可撤销」）
//
// emits:
//   - update:open: 关闭弹窗（由 Modal 触发，含取消、Esc、遮罩、关闭按钮）
//   - confirm:     点击确认按钮时触发
//
// 说明：取消与 Modal 关闭路径统一走 update:open(false)，由父级负责隐藏态语义。

import { Modal, Button } from "@ice-paw/ui";

defineProps<{
  open: boolean;
  title: string;
  message: string;
}>();

const emit = defineEmits<{
  "update:open": [value: boolean];
  confirm: [];
}>();

/** 关闭弹窗：取消 / Esc / 遮罩 / 关闭按钮 */
function handleClose(): void {
  emit("update:open", false);
}

/** 点击确认 */
function handleConfirm(): void {
  emit("confirm");
}
</script>

<template>
  <Modal
    :model-value="open"
    size="sm"
    :title="title"
    @update:model-value="handleClose"
  >
    <p class="confirm-message">{{ message }}</p>

    <template #footer>
      <Button variant="secondary" @click="handleClose">取消</Button>
      <Button variant="danger" @click="handleConfirm">确认</Button>
    </template>
  </Modal>
</template>

<style scoped>
.confirm-message {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-body);
}
</style>