<!--
  AboutSettings — 设置-关于页（只读应用信息）

  展示软件基本身份信息：应用名称 / 版本号 / 标识符（运行时从 tauri.conf.json 读，
  单一真相源）+ 简介 / 仓库 / 许可 / 作者（静态事实）。样式沿用设置页内容卡
  （.settings-card + .field + .form-input.is-readonly，与 GeneralSettings 同款）。
  图标一律 @lucide/vue（Snowflake 品牌标记 / Code2 仓库 / Scale 许可）。
-->
<script setup lang="ts">
import { ref, onMounted } from "vue";
import { Snowflake, Code2, Scale } from "@lucide/vue";
import { bridge } from "../../api/bridge";

interface AppInfo {
  productName: string;
  version: string;
  identifier: string;
}

// 静态产品事实（非运行时数据，不随 tauri.conf.json 走）
const META = {
  description: "本地优先的 LLM 对话工作站",
  repository: "github.com/SakuraSmiles/ice-paw",
  license: "MIT",
  author: "SakuraSmiles",
} as const;

const info = ref<AppInfo>({ productName: "IcePaw", version: "", identifier: "" });
const loading = ref(true);
const loadError = ref<string | null>(null);

onMounted(async () => {
  try {
    info.value = await bridge.appInfo.get();
  } catch (e) {
    console.error("加载应用信息失败:", e);
    loadError.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
});
</script>

<template>
  <div class="settings-content-inner">
    <div class="content-header">
      <h2 class="content-title">关于</h2>
    </div>

    <div class="settings-list">
      <section class="settings-card">
        <div class="card-head">
          <div class="card-head-text">
            <h3 class="card-title">应用信息</h3>
            <p class="card-hint">软件基本身份信息（只读）</p>
          </div>
        </div>

        <!-- 品牌行：标记 + 名称 + 版本 -->
        <div class="brand-row">
          <div class="brand-logo">
            <Snowflake :size="24" />
          </div>
          <div class="brand-text">
            <div class="brand-name">{{ info.productName }}</div>
            <div class="brand-version">v{{ info.version }}</div>
          </div>
        </div>

        <div class="field">
          <div class="field-label">应用名称</div>
          <input :value="info.productName" type="text" class="form-input is-readonly" readonly tabindex="-1" aria-readonly="true" />
        </div>

        <div class="field">
          <div class="field-label">版本号</div>
          <input :value="info.version" type="text" class="form-input is-readonly" placeholder="加载中..." readonly tabindex="-1" aria-readonly="true" />
        </div>

        <div class="field">
          <div class="field-label">标识符</div>
          <input :value="info.identifier" type="text" class="form-input is-readonly" placeholder="加载中..." readonly tabindex="-1" aria-readonly="true" />
        </div>

        <div class="field">
          <div class="field-label">简介</div>
          <input :value="META.description" type="text" class="form-input is-readonly" readonly tabindex="-1" aria-readonly="true" />
        </div>

        <div class="field">
          <div class="field-label">
            <Code2 :size="14" />
            仓库
          </div>
          <input :value="META.repository" type="text" class="form-input is-readonly" readonly tabindex="-1" aria-readonly="true" />
        </div>

        <div class="field">
          <div class="field-label">
            <Scale :size="14" />
            许可
          </div>
          <input :value="META.license" type="text" class="form-input is-readonly" readonly tabindex="-1" aria-readonly="true" />
        </div>

        <div class="field">
          <div class="field-label">作者</div>
          <input :value="META.author" type="text" class="form-input is-readonly" readonly tabindex="-1" aria-readonly="true" />
        </div>

        <div v-if="loadError" class="load-error">版本信息加载失败：{{ loadError }}</div>
      </section>
    </div>
  </div>
</template>

<style scoped>
/* 结构样式与 GeneralSettings 同源（settings-content-inner / content-header /
   content-title / settings-list / settings-card / field / form-input） */
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

.settings-list {
  flex: 1;
  width: 100%;
  padding: 8px 28px 24px;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
  overflow-y: auto;
}

.settings-card {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
  padding: var(--ip-card-padding-md);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-card-radius);
}

.card-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--ip-spacing-2);
}

.card-title {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.card-hint {
  margin: 2px 0 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.5;
}

/* 品牌行 */
.brand-row {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2_5);
  padding: 4px 0 var(--ip-spacing-2);
}

.brand-logo {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 48px;
  height: 48px;
  flex-shrink: 0;
  border-radius: var(--ip-radius-lg);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-primary-500);
}

.brand-text {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.brand-name {
  font-size: var(--ip-text-h2-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  line-height: var(--ip-text-h2-lh);
}

.brand-version {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
}

.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.field-label {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
}

.form-input {
  width: 100%;
  height: var(--ip-input-h-sm);
  padding: 0 10px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  outline: none;
}

.form-input.is-readonly {
  cursor: default;
  color: var(--ip-color-text-secondary);
}

.load-error {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-danger-text);
}
</style>
