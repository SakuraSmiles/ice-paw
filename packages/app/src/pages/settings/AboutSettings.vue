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
const REPO_URL = "https://github.com/SakuraSmiles/ice-paw";

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
      <div class="content-heading">
        <h2 class="content-title">关于</h2>
        <span class="content-en">ABOUT</span>
      </div>
    </div>

    <div class="about-body">
      <div class="bg-word" aria-hidden="true">ICE-PAW</div>
      <div class="reg-frame">
        <i class="reg tl" aria-hidden="true"></i>
        <i class="reg tr" aria-hidden="true"></i>
        <i class="reg bl" aria-hidden="true"></i>
        <i class="reg br" aria-hidden="true"></i>
        <div class="about-content">
          <div class="brand-name">{{ info.productName || "IcePaw" }}</div>
          <div class="brand-sub">本地优先的 AI 多 Agent 桌面工作站</div>
          <div class="brand-sub-en">LOCAL-FIRST / WORKSTATION</div>

          <div class="spec-head">
            <span class="spec-head-label">应用档案</span>
            <span class="spec-head-en">IDENTIFICATION</span>
            <span class="spec-head-line" aria-hidden="true"></span>
          </div>

          <dl class="meta-list">
            <div class="meta-row"><dt>版本</dt><dd>{{ info.version || "…" }}</dd></div>
            <div class="meta-row"><dt>应用标识</dt><dd>{{ info.identifier || "…" }}</dd></div>
            <div class="meta-row">
              <dt>仓库地址</dt>
              <dd><a class="repo-link" :href="REPO_URL" target="_blank" rel="noopener noreferrer">{{ REPOSITORY }}</a></dd>
            </div>
            <div class="meta-row"><dt>开源协议</dt><dd>MIT</dd></div>
          </dl>
          <div class="about-foot">ICEPAW / LOCAL-FIRST AI WORKSTATION</div>
          <div v-if="loadError" class="load-error">版本信息加载失败：{{ loadError }}</div>
        </div>
      </div>
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
  padding: var(--ip-spacing-5) var(--ip-spacing-7) 0;
  flex-shrink: 0;
  height: 72px;
  border-bottom: 1px solid var(--ip-color-border-default);
}

/* 二级菜单内容标题（统一格式）：中文主名 + 英文等宽档案副标（冷青 accent） */
.content-heading {
  display: flex;
  align-items: baseline;
  gap: var(--ip-spacing-3);
}

.content-title {
  position: relative;
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  margin: 0;
}

/* 标题下方分割线左端异色粗线段：宽=标题文字宽（标题是 flex 项，天然 shrink 到文字宽），
   随标题字数自适应；bottom 负值将其从标题下缘伸到 header 底部边框（72px 头高 + 20px 上内边距） */
.content-title::after {
  content: "";
  position: absolute;
  left: 0;
  bottom: -13px;
  width: 100%;
  height: 3px;
  background: var(--ip-accent-dsh);
}

.content-en {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-micro-size);
  letter-spacing: 0.2em;
  color: var(--ip-accent-dsh);
}

/* 内容区：DSH 语言（套准框 + 双语标签 + 等宽档案行 + 冷青点缀 + 水印大字）。
   噪点/暗角/扫描线等演出件不搬阅读面（会把背景染脏）。 */
.about-body {
  flex: 1;
  min-height: 0;
  position: relative;
  overflow: hidden;
  padding: var(--ip-spacing-5) var(--ip-spacing-7) var(--ip-spacing-7);
}

/* 背景巨型水印大字（仅文字母题，不染底色） */
.bg-word {
  position: absolute;
  top: 50%;
  right: -12px;
  transform: translateY(-50%);
  writing-mode: vertical-rl;
  font-size: 110px;
  font-weight: var(--ip-font-weight-bold);
  letter-spacing: 0.14em;
  color: var(--ip-color-text-primary);
  opacity: 0.04;
  white-space: nowrap;
  pointer-events: none;
  user-select: none;
  z-index: 0;
}

/* 套准框：占满内容列（与设置其他页卡片同宽），四角 L 角标贴内容区边缘 */
.reg-frame {
  position: relative;
  z-index: 1;
  width: 100%;
  padding: 40px 44px;
}

.reg {
  position: absolute;
  width: 22px;
  height: 22px;
  border: 0 solid var(--ip-accent-dsh);
  opacity: 0.55;
}
.reg.tl { top: 0; left: 0; border-top-width: 2px; border-left-width: 2px; }
.reg.tr { top: 0; right: 0; border-top-width: 2px; border-right-width: 2px; }
.reg.bl { bottom: 0; left: 0; border-bottom-width: 2px; border-left-width: 2px; }
.reg.br { bottom: 0; right: 0; border-bottom-width: 2px; border-right-width: 2px; }

.about-content { position: relative; }

/* 品牌字标：Orbitron（对齐 splash .boot-logo：weight 640 / ls 0.16em / indent 0.16em） */
.brand-name {
  font-family: 'Orbitron Variable', var(--ip-font-sans);
  font-size: 32px;
  font-weight: 640;
  letter-spacing: 0.16em;
  text-indent: 0.16em;
  color: var(--ip-color-text-primary);
}

.brand-sub {
  margin-top: 14px;
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-medium);
  letter-spacing: 0.04em;
  color: var(--ip-color-text-secondary);
}

.brand-sub-en {
  margin-top: var(--ip-spacing-1_5);
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-micro-size);
  letter-spacing: 0.3em;
  text-indent: 0.3em;
  color: var(--ip-color-text-tertiary);
}

/* 档案区块标题（DSH）：中文主名 + 英文等宽副标 + 右侧 hairline 填补剩余宽度。
   只做结构分区，不重新引入已移走的「1px 线 + 左端粗 accent」分割线。 */
.spec-head {
  display: flex;
  align-items: baseline;
  gap: var(--ip-spacing-3);
  margin-top: var(--ip-spacing-10);
}

.spec-head-label {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  letter-spacing: 0.08em;
  color: var(--ip-color-text-secondary);
}

.spec-head-en {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-micro-size);
  letter-spacing: 0.2em;
  color: var(--ip-accent-dsh);
}

.spec-head-line {
  flex: 1;
  align-self: center;
  height: 1px;
  background: var(--ip-color-border-default);
}

.meta-list {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-4);
  margin: var(--ip-spacing-5) 0 0;
  padding: 0;
}

.meta-row {
  display: flex;
  align-items: baseline;
  gap: var(--ip-spacing-6);
}

.meta-row dt {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  letter-spacing: 0.08em;
  color: var(--ip-accent-dsh);
  flex-shrink: 0;
  width: 72px;
}

.meta-row dd {
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-body-sm-size);
  letter-spacing: 0.04em;
  color: var(--ip-color-text-primary);
  word-break: break-all;
  margin: 0;
}

/* 仓库地址链接：默认主色 + 细下划线，hover 冷青 accent（区别于纯文本行，提示可点） */
.repo-link {
  color: inherit;
  text-decoration: none;
  border-bottom: 1px solid var(--ip-color-border-default);
  transition:
    color var(--ip-duration-fast) var(--ip-ease-out),
    border-color var(--ip-duration-fast) var(--ip-ease-out);
}

.repo-link:hover {
  color: var(--ip-accent-dsh);
  border-bottom-color: var(--ip-accent-dsh);
}

.about-foot {
  margin-top: 40px;
  font-family: var(--ip-font-mono);
  font-size: var(--ip-text-micro-size);
  letter-spacing: 0.24em;
  color: var(--ip-color-text-tertiary);
}

.load-error {
  margin-top: var(--ip-spacing-3);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-danger-text);
}
</style>
