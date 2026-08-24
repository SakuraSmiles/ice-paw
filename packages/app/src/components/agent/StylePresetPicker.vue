<!--
  StylePresetPicker — 风格预设选择弹层（创建/编辑两用）

  两层设计（docs/agent-prompt-draft.md）：预设是素材不是档位——全文展示让用户
  看清自己将拿到什么；选完即完成使命，落盘后就是用户自己的文本。

  交互（2026-08-23 第三轮）：胶囊 tab 切换浏览 + 底部显式确认——浏览与决定
  分离，点胶囊只换内容不做任何选择动作。
    mode=create：确认=「使用该风格」（随表单保存写入）；已选档胶囊带 ✓，
                可「清除选择」回默认通用句
    mode=edit：  确认=「插入到 agent.yaml」；已有非空非出生默认句时先出覆盖
                确认横幅（显示现有首行），再点「覆盖写入」才生效

  Props:
    agentName      {name} 填充用（表单当前名称）
    selectedId     create 模式当前已选档 id
    existingPrompt edit 模式现有 system_prompt（null=明确无值 / undefined=读取失败未知→保守确认）
    inserting      edit 模式写入中（按钮禁用）
    error          写入错误（edit 模式由父级 bridge 调用回传）
  Emits: close / select(preset|null)（create）/ pick(preset)（edit，覆盖确认已过）
-->
<script setup lang="ts">
import { ref, computed } from "vue";
import { useEscapeStack } from "../../composables/useEscapeStack";
import { STYLE_PRESETS, fillPresetName, isBirthDefaultPrompt, type StylePreset } from "../../data/stylePresets";

const props = defineProps<{
  mode: "create" | "edit";
  agentName: string;
  selectedId?: string | null;
  existingPrompt?: string | null;
  inserting?: boolean;
  error?: string;
}>();

const emit = defineEmits<{
  close: [];
  select: [preset: StylePreset | null];
  pick: [preset: StylePreset];
}>();

useEscapeStack(() => emit("close"));

/** 当前浏览档（打开时定位到已选档，无已选从第一档起） */
const activeId = ref<string>(STYLE_PRESETS.some((p) => p.id === props.selectedId) ? props.selectedId! : STYLE_PRESETS[0].id);
const activePreset = computed(
  () => STYLE_PRESETS.find((p) => p.id === activeId.value) ?? STYLE_PRESETS[0],
);

/** 覆盖确认横幅态（edit；切档即复位） */
const confirming = ref(false);

/** 覆盖确认判据：明确无值免、出生默认句免（最常见操作，拦一道是噪音）、其余（含未知）确认 */
function needsConfirm(): boolean {
  if (props.mode !== "edit") return false;
  const cur = props.existingPrompt;
  if (cur === null) return false;
  if (cur === undefined) return true;
  return !isBirthDefaultPrompt(cur, props.agentName);
}

/** 现有内容首行（覆盖确认里展示，让用户认出自己写的东西） */
const existingFirstLine = computed(
  () => (props.existingPrompt ?? "").split("\n").find((l) => l.trim()) || "（空）",
);

/** 全文预览（{name} 已替换——所见即落盘） */
function previewText(p: StylePreset): string {
  return fillPresetName(p.text, props.agentName);
}

function onTab(p: StylePreset) {
  activeId.value = p.id;
  confirming.value = false;
}

/** tablist 左右方向键循环切换（tabs 模式标准键盘行为） */
function onTabsKeydown(e: KeyboardEvent) {
  if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
  e.preventDefault();
  const i = STYLE_PRESETS.findIndex((p) => p.id === activeId.value);
  const next = e.key === "ArrowRight" ? i + 1 : i - 1;
  onTab(STYLE_PRESETS[(next + STYLE_PRESETS.length) % STYLE_PRESETS.length]);
}

/** 底部主按钮：浏览与确认分离——点胶囊只换内容，确认走这里 */
function onConfirm() {
  if (props.inserting) return;
  if (needsConfirm()) {
    confirming.value = true;
    return;
  }
  confirming.value = false;
  if (props.mode === "create") emit("select", activePreset.value);
  else emit("pick", activePreset.value);
}
</script>

<template>
  <Teleport to="body">
    <div class="sp-backdrop" @click.self="emit('close')">
      <div class="sp-modal" role="dialog" aria-label="选择风格预设">
        <div class="sp-head">
          <h4 class="sp-title">风格预设</h4>
          <button type="button" class="sp-close" title="关闭" @click="emit('close')">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
          </button>
        </div>

        <!-- 胶囊 tab：点=切换浏览，不是选择动作 -->
        <div class="sp-tabs" role="tablist" @keydown="onTabsKeydown">
          <button
            v-for="p in STYLE_PRESETS"
            :key="p.id"
            type="button"
            role="tab"
            class="sp-tab"
            :class="{ active: activeId === p.id }"
            :aria-selected="activeId === p.id"
            :title="mode === 'create' && selectedId === p.id ? '当前已选' : undefined"
            @click="onTab(p)"
          >
            {{ p.name }}
            <svg
              v-if="mode === 'create' && selectedId === p.id"
              class="sp-tab-check"
              width="13" height="13" viewBox="0 0 24 24" fill="none"
              stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"
            ><polyline points="20 6 9 17 4 12" /></svg>
          </button>
        </div>

        <!-- 当前档单片内容：一句适用说明 + 全文 -->
        <div class="sp-body" role="tabpanel">
          <p class="sp-note">{{ activePreset.note }}</p>
          <pre class="sp-text">{{ previewText(activePreset) }}</pre>
        </div>

        <p v-if="error" class="sp-error">{{ error }}</p>

        <div class="sp-foot">
          <!-- edit 覆盖确认横幅态 -->
          <template v-if="confirming">
            <span class="sp-confirm-text">将覆盖现有 system_prompt（首行：{{ existingFirstLine }}）</span>
            <div class="sp-actions">
              <button type="button" class="sp-btn ghost" @click="confirming = false">返回</button>
              <button type="button" class="sp-btn primary" :disabled="inserting" @click="emit('pick', activePreset)">
                {{ inserting ? "写入中…" : "覆盖写入" }}
              </button>
            </div>
          </template>
          <template v-else>
            <div class="sp-actions">
              <button
                v-if="mode === 'create' && selectedId"
                type="button"
                class="sp-btn ghost"
                @click="emit('select', null)"
              >清除选择</button>
              <button type="button" class="sp-btn primary" :disabled="inserting" @click="onConfirm">
                {{ mode === "create" ? "使用该风格" : "插入到 agent.yaml" }}
              </button>
            </div>
          </template>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.sp-backdrop {
  position: fixed;
  inset: 0;
  z-index: var(--ip-z-modal-overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--ip-color-bg-overlay);
}
.sp-modal {
  display: flex;
  flex-direction: column;
  width: 600px;
  max-width: calc(100vw - 48px);
  max-height: calc(100vh - 96px);
  padding: var(--ip-spacing-4);
  border-radius: var(--ip-radius-lg);
  background: var(--ip-color-bg-primary);
  box-shadow: var(--ip-shadow-lg);
}

/* 标题行：标题 + 右上关闭（关闭走 ✕/Esc/backdrop，不占底部按钮位） */
.sp-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--ip-spacing-3);
}
.sp-title {
  font-size: var(--ip-text-h3-size);
  font-weight: 600;
  color: var(--ip-color-text-primary);
  margin: 0;
}
.sp-close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border: none;
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-text-tertiary);
  background: transparent;
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.sp-close:hover {
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
}

/* 胶囊 tab */
.sp-tabs {
  display: flex;
  gap: var(--ip-spacing-2);
  margin-bottom: var(--ip-spacing-3);
}
.sp-tab {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  height: 30px;
  padding: 0 16px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: 500;
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-tertiary);
  border: 1px solid transparent;
  border-radius: var(--ip-radius-full);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.sp-tab:hover {
  color: var(--ip-color-text-primary);
}
.sp-tab.active {
  color: var(--ip-primary-600);
  background: var(--ip-color-primary-soft-bg);
  border-color: var(--ip-primary-300);
}
.sp-tab-check {
  color: var(--ip-primary-600);
}

/* 内容区：适用说明一句 + 全文（单片完整展示，长文区内滚动） */
.sp-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-3) var(--ip-spacing-4);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  background: var(--ip-color-bg-secondary);
}
.sp-note {
  margin: 0;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
}
.sp-text {
  flex: 1;
  margin: 0;
  font-family: var(--ip-font-sans);
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  line-height: 1.7;
  white-space: pre-wrap;
  word-break: break-word;
  overflow-y: auto;
}

.sp-error {
  margin: var(--ip-spacing-2_5) 0 0;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-danger-text);
  word-break: break-all;
}

/* 底部：确认横幅态独占整行，默认态仅右对齐按钮 */
.sp-foot {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: var(--ip-spacing-3);
  margin-top: var(--ip-spacing-3);
}
.sp-confirm-text {
  flex: 1;
  min-width: 0;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-warning-text);
  word-break: break-all;
}
.sp-actions {
  display: flex;
  gap: var(--ip-spacing-2);
  flex-shrink: 0;
}
.sp-btn {
  height: 30px;
  padding: 0 16px;
  border: none;
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-body-sm-size);
  font-weight: 500;
  cursor: pointer;
}
.sp-btn.primary {
  background: var(--ip-primary-500);
  color: #fff;
}
.sp-btn.primary:disabled {
  opacity: 0.6;
  cursor: default;
}
.sp-btn.ghost {
  background: transparent;
  border: 1px solid var(--ip-color-border-default);
  color: var(--ip-color-text-secondary);
}
.sp-btn.ghost:hover {
  color: var(--ip-primary-600);
  border-color: var(--ip-primary-400);
}
</style>
