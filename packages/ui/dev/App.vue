<script setup lang="ts">
// IcePaw UI 预览站根组件 — 阶段 1 最小骨架
</script>

<template>
  <div class="preview-root">
    <header class="preview-header">
      <h1>IcePaw UI · 预览站</h1>
      <p class="preview-subtitle">组件库骨架已搭建，阶段 1 无可预览组件</p>
    </header>
    <main class="preview-main">
      <div class="preview-empty">
        <p>🎉 Monorepo 骨架搭建完成</p>
        <p>后续阶段将在此展示所有组件</p>
      </div>
    </main>
  </div>
</template>

<style scoped>
.preview-root {
  min-height: 100vh;
  display: flex;
  flex-direction: column;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  color: var(--ip-color-text-primary, #111827);
  background: var(--ip-color-bg-secondary, #f9fafb);
}

.preview-header {
  padding: var(--ip-spacing-xl, 24px) var(--ip-spacing-2xl, 32px);
  border-bottom: 1px solid var(--ip-color-border, #e5e7eb);
  background: var(--ip-color-bg-primary, #fff);
}

.preview-header h1 {
  margin: 0;
  font-size: var(--ip-font-size-2xl, 24px);
  font-weight: var(--ip-font-weight-semibold, 600);
}

.preview-subtitle {
  margin: var(--ip-spacing-xs, 4px) 0 0;
  color: var(--ip-color-text-secondary, #6b7280);
  font-size: var(--ip-font-size-sm, 14px);
}

.preview-main {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}

.preview-empty {
  text-align: center;
  color: var(--ip-color-text-secondary, #6b7280);
}

.preview-empty p {
  margin: var(--ip-spacing-sm, 8px) 0;
}

.preview-empty p:first-child {
  font-size: var(--ip-font-size-xl, 20px);
  font-weight: var(--ip-font-weight-medium, 500);
}
</style>
