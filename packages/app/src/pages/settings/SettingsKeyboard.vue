<script setup lang="ts">
import { ref } from "vue";

const shortcuts = ref([
  { action: "发送消息", key: "Enter" },
  { action: "换行", key: "Shift + Enter" },
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
        </tr>
      </thead>
      <tbody>
        <tr v-for="item in shortcuts" :key="item.action">
          <td class="col-action">{{ item.action }}</td>
          <td class="col-key"><kbd class="kbd">{{ item.key }}</kbd></td>
        </tr>
      </tbody>
    </table>
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
.kbd {
  display: inline-block; padding: var(--ip-spacing-0_5) var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size); font-family: var(--ip-font-mono);
  color: var(--ip-color-text-secondary); background-color: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default); border-radius: var(--ip-radius-sm);
}
</style>
