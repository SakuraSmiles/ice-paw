<!--
  AboutSettings — 设置-关于页（只读应用信息）

  极简形态（2026-09-17 用户拍板）：无卡片/无边框/无表单，内容直接铺在页面底色上，
  多行纯文本展示三项——软件名称 / 版本号 / 仓库地址。版本号运行时从 tauri.conf.json
  读（单一真相源）。二级菜单布局归 SettingsLayout 管，本页只负责内容区。
-->
<script setup lang="ts">
import { ref, onMounted } from "vue";
import { bridge } from "../../api/bridge";

interface AppInfo {
  productName: string;
  version: string;
  identifier: string;
}

const info = ref<AppInfo>({ productName: "IcePaw", version: "", identifier: "" });
const loadError = ref<string | null>(null);

const REPOSITORY = "github.com/SakuraSmiles/ice-paw";

onMounted(async () => {
  try {
    info.value = await bridge.appInfo.get();
  } catch (e) {
    console.error("加载应用信息失败:", e);
    loadError.value = e instanceof Error ? e.message : String(e);
  }
});
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <h2 class="content-title">关于</h2>
    </div>

    <div class="about-body">
      <div class="about-name">{{ info.productName }}</div>
      <div class="about-version">版本 {{ info.version || "…" }}</div>
      <div class="about-repo">{{ REPOSITORY }}</div>
      <div v-if="loadError" class="load-error">版本信息加载失败：{{ loadError }}</div>
    </div>
  </div>
</template>

<style scoped>
.settings-content-inner {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.content-header {
  display: flex;
  align-items: center;
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

/* 内容区：直接底色 + 多行纯文本（无卡片无边框），左对齐贴内容列 */
.about-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  padding: var(--ip-spacing-4) 28px var(--ip-spacing-4);
  overflow-y: auto;
}

.about-name {
  font-size: var(--ip-text-h2-size);
  line-height: var(--ip-text-h2-lh);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.about-version {
  margin-top: var(--ip-spacing-2);
  font-size: var(--ip-text-body-size);
  line-height: var(--ip-text-body-lh);
  color: var(--ip-color-text-secondary);
}

.about-repo {
  margin-top: var(--ip-spacing-1);
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
}

.load-error {
  margin-top: var(--ip-spacing-3);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-danger-text);
}
</style>
