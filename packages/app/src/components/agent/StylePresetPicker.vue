<!--
  StylePresetPicker — 风格预设选择弹层（创建/编辑两用）

  两层设计（docs/agent-prompt-draft.md）：预设是素材不是档位——全文展示让用户
  看清自己将拿到什么；选完即完成使命，落盘后就是用户自己的文本。

  mode=create：点卡=选中（已选卡再点=取消），随表单保存写入 NewAgent payload；
  mode=edit：  点卡=插入（写 agent.yaml）；已有非空非出生默认句时卡片翻就地
              覆盖确认态（显示现有首行），确认后才 emit pick。

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

/** 进入覆盖确认态的档 id（MoreMenu 同款就地二次确认） */
const confirmingId = ref<string | null>(null);

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

function onCardClick(p: StylePreset) {
  if (props.inserting) return;
  if (props.mode === "create") {
    emit("select", props.selectedId === p.id ? null : p);
    return;
  }
  if (needsConfirm() && confirmingId.value !== p.id) {
    confirmingId.value = p.id;
    return;
  }
  confirmingId.value = null;
  emit("pick", p);
}
</script>

<template>
  <Teleport to="body">
    <div class="sp-backdrop" @click.self="emit('close')">
      <div class="sp-modal" role="dialog" aria-label="选择风格预设">
        <h4 class="sp-title">
          风格预设
          <span class="sp-sub">{{
            mode === "create" ? "选一套作为起点，保存时写入" : "选一套插入 agent.yaml，之后可自由修改"
          }}</span>
        </h4>

        <div class="sp-grid">
          <div
            v-for="p in STYLE_PRESETS"
            :key="p.id"
            class="sp-card"
            :class="{ selected: mode === 'create' && selectedId === p.id }"
            role="button"
            tabindex="0"
            @click="onCardClick(p)"
            @keydown.enter.prevent="onCardClick(p)"
          >
            <div class="sp-card-head">
              <span class="sp-name">{{ p.name }}</span>
              <!-- 选中标记（create） -->
              <svg
                v-if="mode === 'create' && selectedId === p.id"
                class="sp-check"
                width="16" height="16" viewBox="0 0 24 24" fill="none"
                stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"
              ><circle cx="12" cy="12" r="10" /><polyline points="16 8.5 11 13.5 8 10.5" /></svg>
            </div>
            <p class="sp-note">{{ p.note }}</p>
            <pre class="sp-text">{{ previewText(p) }}</pre>

            <!-- edit 覆盖确认态（就地翻面；点事件不再冒泡触发卡片） -->
            <div v-if="mode === 'edit' && confirmingId === p.id" class="sp-confirm" @click.stop>
              <span class="sp-confirm-text">将覆盖现有 system_prompt（首行：{{ existingFirstLine }}）</span>
              <div class="sp-confirm-actions">
                <button type="button" class="sp-btn primary" :disabled="inserting" @click="emit('pick', p)">
                  {{ inserting ? "写入中…" : "覆盖写入" }}
                </button>
                <button type="button" class="sp-btn ghost" @click="confirmingId = null">取消</button>
              </div>
            </div>
            <div v-else-if="mode === 'edit'" class="sp-card-foot">点击插入到 agent.yaml</div>
          </div>
        </div>

        <p v-if="error" class="sp-error">{{ error }}</p>

        <div class="sp-foot">
          <span class="sp-foot-note">预设是起点，不是档位——插入后就是你的文本，可直接在 agent.yaml 里修改</span>
          <div class="sp-foot-actions">
            <button
              v-if="mode === 'create' && selectedId"
              type="button"
              class="sp-btn ghost"
              @click="emit('select', null)"
            >清除选择（用默认）</button>
            <button type="button" class="sp-btn ghost" @click="emit('close')">关闭</button>
          </div>
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
  width: 780px;
  max-width: calc(100vw - 48px);
  max-height: calc(100vh - 96px);
  overflow-y: auto;
  padding: var(--ip-spacing-4);
  border-radius: var(--ip-radius-lg);
  background: var(--ip-color-bg-primary);
  box-shadow: var(--ip-shadow-lg);
}
.sp-title {
  font-size: var(--ip-text-body-lg-size);
  font-weight: 600;
  color: var(--ip-color-text-primary);
  margin: 0 0 var(--ip-spacing-3);
}
.sp-sub {
  font-weight: 400;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
  margin-left: var(--ip-spacing-2);
}

.sp-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(210px, 1fr));
  gap: var(--ip-spacing-3);
}
.sp-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: var(--ip-spacing-3);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  background: var(--ip-color-bg-primary);
  cursor: pointer;
  transition: border-color var(--ip-duration-fast) var(--ip-ease-out);
}
.sp-card:hover {
  border-color: var(--ip-color-border-focus);
}
.sp-card.selected {
  border-color: var(--ip-primary-500);
  background: var(--ip-color-primary-soft-bg);
}
.sp-card:focus-visible {
  outline: 2px solid var(--ip-color-border-focus);
  outline-offset: 1px;
}
.sp-card-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.sp-name {
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.sp-check {
  color: var(--ip-primary-600);
  flex-shrink: 0;
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
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-secondary);
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 260px;
  overflow-y: auto;
}
.sp-card-foot {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
  text-align: center;
}

/* 覆盖确认态（卡片底部就地展开） */
.sp-confirm {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}
.sp-confirm-text {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-warning-text);
  word-break: break-all;
}
.sp-confirm-actions {
  display: flex;
  gap: var(--ip-spacing-2);
}

.sp-error {
  margin: var(--ip-spacing-2_5) 0 0;
  font-size: var(--ip-text-micro-size);
  color: var(--ip-danger-text);
  word-break: break-all;
}

.sp-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-3);
  margin-top: var(--ip-spacing-3);
}
.sp-foot-note {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
}
.sp-foot-actions {
  display: flex;
  gap: var(--ip-spacing-2);
  flex-shrink: 0;
}
.sp-btn {
  height: 28px;
  padding: 0 14px;
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
