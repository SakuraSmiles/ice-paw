<script setup lang="ts">
// KbSettings.vue — 全局知识库（二级菜单页）
// 与「智能体 / 工具集」同级。内容复用统一组件 KbDocumentList（global）。
import KbDocumentList from "../../components/kb/KbDocumentList.vue";
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <div class="content-heading">
        <h2 class="content-title">全局知识库</h2>
        <span class="content-en">KNOWLEDGE</span>
      </div>
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
  padding: var(--ip-spacing-5) var(--ip-spacing-7) 0;
  flex-shrink: 0;
  height: 72px;
  border-bottom: 1px solid var(--ip-color-border-default);
}

.content-title {
  position: relative;
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin: 0;
}

/* 标题下方分割线左端异色粗线段：宽=标题文字宽（自适应字数） */
.content-title::after {
  content: "";
  position: absolute;
  left: 0;
  bottom: -13px;
  width: 100%;
  height: 3px;
  background: var(--ip-accent-dsh);
}

.content-heading {
  display: flex;
  align-items: baseline;
  gap: var(--ip-spacing-3);
}

.content-en {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-micro-size);
  letter-spacing: 0.2em;
  color: var(--ip-accent-dsh);
}

.kb-page-content {
  flex: 1;
  overflow-y: auto;
  padding: var(--ip-spacing-5) var(--ip-spacing-7) var(--ip-spacing-6);
  min-height: 0;
}
</style>
