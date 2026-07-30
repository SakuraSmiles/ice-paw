<script setup lang="ts">
// KbSettings.vue — 全局知识库（二级菜单页）
// 与「智能体 / MCP 工具」同级。内容复用统一组件 KbDocumentList（global）。
import KbDocumentList from "../../components/kb/KbDocumentList.vue";
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <h2 class="content-title">全局知识库</h2>
    </div>

    <div class="kb-page-content">
      <KbDocumentList scope="global" />
    </div>
  </div>
</template>

<style scoped>
/* 与 McpSettings / AgentSettings 的内容区头部风格保持一致 */
.settings-content-inner {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 0;
  min-height: 0;
}

.content-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 20px 28px 0;
  flex-shrink: 0;
  height: 56px;
}

.content-title {
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin: 0;
}

.kb-page-content {
  flex: 1;
  overflow-y: auto;
  padding: 8px 28px 24px;
  min-height: 0;
}
</style>
