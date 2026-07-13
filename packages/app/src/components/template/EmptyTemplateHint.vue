<script setup lang="ts">
// 模板空状态提示组件
//
// 职责：
//   - 模板列表为空时显示欢迎引导
//   - 提供「创建模板」入口按钮

import { Button } from "@ice-paw/ui";
import { Plus } from "lucide-vue-next";

const emit = defineEmits<{
  create: [];
}>();
</script>

<template>
  <div class="empty-hint">
    <h2 class="empty-title">还没有模板</h2>
    <p class="empty-desc">
      模板是「带变量占位符的 system prompt + user prompt 前缀」组合。<br />
      在聊天中通过 @模板名 或点击芯片注入，可大幅提升重复性任务效率。
    </p>
    <Button variant="primary" size="md" @click="emit('create')">
      <template #icon-left>
        <Plus :size="16" aria-hidden="true" />
      </template>
      创建模板
    </Button>
  </div>
</template>

<style scoped>
.empty-hint {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--ip-spacing-12) var(--ip-spacing-6);
  text-align: center;
  gap: var(--ip-spacing-4);
}

.empty-title {
  margin: 0;
  font-size: var(--ip-text-h2-size);
  font-weight: var(--ip-font-weight-semibold);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-primary);
}

.empty-desc {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-tertiary);
  max-width: 420px;
}
</style>
