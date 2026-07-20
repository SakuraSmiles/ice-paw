<script setup lang="ts">
import { ref } from "vue";

const shortcuts = ref([
  { action: "新建会话", key: "Ctrl/Cmd + N" },
  { action: "搜索会话", key: "Ctrl/Cmd + K" },
  { action: "切换 Agent", key: "Ctrl/Cmd + Shift + A" },
  { action: "发送消息", key: "Enter" },
  { action: "换行", key: "Shift + Enter" },
  { action: "停止生成", key: "Ctrl/Cmd + ." },
  { action: "打开设置", key: "Ctrl/Cmd + ," },
  { action: "关闭弹窗", key: "Esc" },
]);
</script>

<template>
  <div class="settings-keyboard">
    <h2 class="section-title">快捷键</h2>
    <table class="shortcut-table">
      <thead>
        <tr>
          <th class="col-action">功能</th>
          <th class="col-key">快捷键</th>
          <th class="col-btn" />
        </tr>
      </thead>
      <tbody>
        <tr v-for="item in shortcuts" :key="item.action">
          <td class="col-action">{{ item.action }}</td>
          <td class="col-key"><kbd class="kbd">{{ item.key }}</kbd></td>
          <td class="col-btn">
            <button class="btn-sm" type="button" disabled>重录</button>
          </td>
        </tr>
      </tbody>
    </table>
    <div class="action-row">
      <button class="btn-secondary" type="button" disabled>恢复默认</button>
    </div>
  </div>
</template>

<style scoped>
.settings-keyboard { max-width: 640px; }
.section-title {
  font-size: var(--ip-text-heading-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin-bottom: var(--ip-spacing-4);
}
.shortcut-table { width: 100%; border-collapse: collapse; font-size: var(--ip-text-body-size); }
.shortcut-table th {
  text-align: left; font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-tertiary); font-size: var(--ip-text-caption-size);
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  border-bottom: 1px solid var(--ip-color-border-default);
}
.shortcut-table td {
  padding: var(--ip-spacing-3) var(--ip-spacing-3);
  border-bottom: 1px solid var(--ip-color-border-subtle);
  color: var(--ip-color-text-primary);
}
.col-action { width: 40%; }
.col-key { width: auto; }
.col-btn { width: 80px; text-align: right; }
.kbd {
  display: inline-block; padding: var(--ip-spacing-0_5) var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size); font-family: var(--ip-font-mono);
  color: var(--ip-color-text-secondary); background-color: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-sm);
}
.action-row { margin-top: var(--ip-spacing-6); display: flex; justify-content: flex-end; }
.btn-sm {
  display: inline-flex; align-items: center; justify-content: center;
  height: var(--ip-btn-h-xs); padding: 0 var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size); font-family: inherit;
  color: var(--ip-color-text-secondary); background: transparent;
  border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-sm);
  cursor: pointer; transition: var(--ip-transition-colors);
}
.btn-sm:hover:not(:disabled) { color: var(--ip-color-text-primary); border-color: var(--ip-color-border-hover); }
.btn-sm:disabled { opacity: 0.5; cursor: not-allowed; }
.btn-secondary {
  display: inline-flex; align-items: center; justify-content: center;
  height: var(--ip-btn-h-sm); padding: 0 var(--ip-spacing-3);
  font-size: var(--ip-text-body-sm-size); font-family: inherit;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary); background: transparent;
  border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-btn-radius);
  cursor: pointer; transition: var(--ip-transition-colors);
}
.btn-secondary:hover:not(:disabled) {
  color: var(--ip-color-text-primary); border-color: var(--ip-color-border-hover);
  background-color: var(--ip-color-bg-hover);
}
.btn-secondary:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
