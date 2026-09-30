<script setup lang="ts">
// AskUserCard.vue — 用户选择内联卡（ask_user，2026-09-30 激活会话分支）
//
// 渲染激活会话的待处理选择请求（chat.activeConvAskRequest），输入框上方弹出
// （与 AuthRequestCard 同注意力位）。agent 在需要拍板的分叉点调 ask_user
// 工具 → 此卡出现 → 用户点选 → respondToAsk 唤醒后端常驻等待的回合。
//
// 交互形态（拍板 2026-09-30）：
// - 单选：选项按钮点击即答（无确认步——一词级决策不添摩擦）
// - 多选：勾选 + 「提交」按钮
// - allow_custom：「其他…」展开自由输入（开放式答案的逃生舱，不只有选择题）
// - 「跳过」= dismissed：agent 收到「自行决策并说明未经确认」
// - 常驻等待无倒计时（对比授权卡 120s）——仅用户停止生成时后端 cancel 清卡
import { ref, computed, watch, nextTick } from "vue";
import { ListChecks, CornerDownLeft } from "@lucide/vue";
import { useChatStore } from "../../stores/chat";

const chat = useChatStore();

const entry = computed(() => chat.activeConvAskRequest);
const req = computed(() => entry.value?.payload ?? null);

// ---- 作答草稿（新请求到达时重置；同授权卡 scope 重置模式）----
const selected = ref<Set<string>>(new Set());
const customOpen = ref(false);
const customText = ref("");
const customInput = ref<HTMLInputElement | null>(null);
watch(
  () => req.value?.request_id,
  () => {
    selected.value = new Set();
    customOpen.value = false;
    customText.value = "";
  },
);

function toggleOption(label: string) {
  if (!req.value?.multiple) return; // 单选不走勾选态（点击即答）
  const m = new Set(selected.value);
  if (m.has(label)) m.delete(label);
  else m.add(label);
  selected.value = m;
}

async function answerSingle(label: string) {
  if (!req.value) return;
  await chat.respondToAsk(req.value.request_id, "answered", [label], null);
}

async function openCustom() {
  customOpen.value = true;
  await nextTick();
  customInput.value?.focus();
}

/** 多选提交（勾选项 + 自由输入至少一样才可提交） */
const canSubmitMultiple = computed(() =>
  selected.value.size > 0 || customText.value.trim().length > 0,
);
async function submitMultiple() {
  if (!req.value || !canSubmitMultiple.value) return;
  await chat.respondToAsk(
    req.value.request_id,
    "answered",
    [...selected.value],
    customText.value.trim() || null,
  );
}

/** 单选「其他」提交 */
async function submitCustom() {
  if (!req.value || !customText.value.trim()) return;
  await chat.respondToAsk(
    req.value.request_id,
    "answered",
    [],
    customText.value.trim(),
  );
}

async function dismiss() {
  if (!req.value) return;
  await chat.respondToAsk(req.value.request_id, "dismissed", [], null);
}
</script>

<template>
  <Transition name="auth-card">
    <div v-if="req" class="ask-card" role="alertdialog" aria-label="agent 提问，等待你的选择">
      <div class="ask-main">
        <!-- L1 问题（图标 + 问题本体，白空间代替标题行） -->
        <div class="ask-line1">
          <ListChecks :size="14" class="ask-icon" aria-hidden="true" />
          <span class="ask-question" :title="req.question">{{ req.question }}</span>
        </div>

        <!-- L2 选项区：单选 = 按钮点击即答；多选 = 勾选 + 提交 -->
        <div
          v-if="!req.multiple"
          class="ask-options"
          role="radiogroup"
          :aria-label="req.question"
        >
          <button
            v-for="opt in req.options"
            :key="opt.label"
            type="button"
            class="ask-opt"
            role="radio"
            :aria-checked="false"
            @click="answerSingle(opt.label)"
          >
            <span class="ask-opt-label">{{ opt.label }}</span>
            <span v-if="opt.description" class="ask-opt-desc">{{ opt.description }}</span>
          </button>
        </div>
        <div v-else class="ask-options" role="group" :aria-label="req.question">
          <button
            v-for="opt in req.options"
            :key="opt.label"
            type="button"
            class="ask-opt"
            :class="{ active: selected.has(opt.label) }"
            role="checkbox"
            :aria-checked="selected.has(opt.label)"
            @click="toggleOption(opt.label)"
          >
            <span class="ask-check" :class="{ on: selected.has(opt.label) }" aria-hidden="true" />
            <span class="ask-opt-label">{{ opt.label }}</span>
            <span v-if="opt.description" class="ask-opt-desc">{{ opt.description }}</span>
          </button>
        </div>

        <!-- 自由输入逃生舱（allow_custom）：单选=「其他…」按钮展开；多选=常驻输入行 -->
        <div v-if="req.allow_custom" class="ask-custom">
          <template v-if="!req.multiple">
            <div v-if="!customOpen" class="ask-custom-row">
              <button type="button" class="ask-other-btn" @click="openCustom">其他…</button>
            </div>
            <div v-else class="ask-custom-row">
              <input
                ref="customInput"
                v-model="customText"
                class="ask-input"
                type="text"
                placeholder="输入你的回答"
                aria-label="自定义回答"
                @keydown.enter.prevent="submitCustom"
              />
              <button
                type="button"
                class="ask-submit-btn"
                :disabled="!customText.trim()"
                @click="submitCustom"
              >
                <CornerDownLeft :size="13" aria-hidden="true" />提交
              </button>
            </div>
          </template>
          <div v-else class="ask-custom-row">
            <input
              v-model="customText"
              class="ask-input"
              type="text"
              placeholder="或输入补充（可选）"
              aria-label="自定义补充"
              @keydown.enter.prevent="submitMultiple"
            />
          </div>
        </div>

        <!-- L3 收束行：多选提交 + 跳过 -->
        <div class="ask-line3">
          <span class="ask-hint">{{ req.multiple ? "可多选" : "点击选项即作答" }}</span>
          <div class="ask-actions">
            <button
              v-if="req.multiple"
              class="ask-btn ask-btn-primary"
              :disabled="!canSubmitMultiple"
              @click="submitMultiple"
            >提交</button>
            <button class="ask-btn ask-btn-skip" @click="dismiss">跳过</button>
          </div>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
/* 容器与 AuthRequestCard 同族：宽度对齐输入框列、悬于输入框正上方 */
.ask-card {
  margin: 0 auto var(--ip-spacing-2);
  width: min(calc(100% - 48px), 800px);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-lg);
  flex-shrink: 0;
}

.ask-main {
  padding: var(--ip-spacing-2_5) var(--ip-spacing-3);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}

/* L1：问题行 */
.ask-line1 {
  display: flex;
  align-items: flex-start;
  gap: var(--ip-spacing-1_5);
  min-width: 0;
}
.ask-icon { color: var(--ip-primary-600); flex-shrink: 0; margin-top: 2px; }
.ask-question {
  flex: 1;
  min-width: 0;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  line-height: 1.5;
  overflow-wrap: anywhere;
}

/* 选项区 */
.ask-options {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1_5);
}
.ask-opt {
  display: flex;
  align-items: baseline;
  gap: var(--ip-spacing-2);
  text-align: left;
  padding: var(--ip-spacing-1_5) var(--ip-spacing-2_5);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  background: var(--ip-color-bg-secondary);
  cursor: pointer;
  font: inherit;
  color: inherit;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.ask-opt:hover { border-color: var(--ip-primary-400); background: var(--ip-primary-soft-bg); }
.ask-opt:focus-visible { outline: 2px solid var(--ip-primary-500); outline-offset: 1px; }
.ask-opt.active {
  border-color: var(--ip-primary-500);
  background: var(--ip-primary-soft-bg);
}
/* 勾选方块（多选）——纯 CSS，选中打勾用边框旋转 */
.ask-check {
  flex-shrink: 0;
  width: 14px;
  height: 14px;
  border: 1.5px solid var(--ip-color-border-strong, var(--ip-color-text-tertiary));
  border-radius: 3px;
  position: relative;
  top: 2px;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.ask-check.on {
  background: var(--ip-primary-500);
  border-color: var(--ip-primary-500);
}
.ask-check.on::after {
  content: "";
  position: absolute;
  left: 3.5px;
  top: 0.5px;
  width: 4px;
  height: 8px;
  border: solid white;
  border-width: 0 1.5px 1.5px 0;
  transform: rotate(40deg);
}
.ask-opt-label {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  overflow-wrap: anywhere;
}
.ask-opt-desc {
  flex: 1;
  min-width: 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  overflow-wrap: anywhere;
}

/* 自由输入行 */
.ask-custom-row {
  display: flex;
  gap: var(--ip-spacing-1_5);
  align-items: center;
}
.ask-other-btn {
  padding: 4px var(--ip-spacing-2_5);
  border: none;
  border-radius: var(--ip-radius-md);
  background: none;
  color: var(--ip-color-text-tertiary);
  font-size: var(--ip-text-caption-size);
  cursor: pointer;
  align-self: flex-start;
}
.ask-other-btn:hover { color: var(--ip-primary-600); background: var(--ip-color-bg-tertiary); }
.ask-input {
  flex: 1;
  min-width: 0;
  padding: 5px var(--ip-spacing-2_5);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  background: var(--ip-color-bg-secondary);
  font: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
}
.ask-input:focus-visible { outline: 2px solid var(--ip-primary-500); outline-offset: 1px; }
.ask-submit-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 5px var(--ip-spacing-2_5);
  border: none;
  border-radius: var(--ip-radius-md);
  background: var(--ip-primary-500);
  color: white;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  cursor: pointer;
}
.ask-submit-btn:disabled { opacity: 0.5; cursor: not-allowed; }
.ask-submit-btn svg { display: inline-block; }

/* L3 收束行 */
.ask-line3 {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-2);
}
.ask-hint {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}
.ask-actions {
  display: flex;
  gap: var(--ip-spacing-1_5);
}
.ask-btn {
  padding: 5px var(--ip-spacing-3);
  border-radius: var(--ip-radius-md);
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  cursor: pointer;
  border: none;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.ask-btn-primary { background: var(--ip-primary-500); color: white; }
.ask-btn-primary:disabled { opacity: 0.5; cursor: not-allowed; }
.ask-btn-primary:not(:disabled):hover { opacity: 0.9; }
.ask-btn-skip { background: var(--ip-color-bg-tertiary); color: var(--ip-color-text-secondary); }
.ask-btn-skip:hover { background: var(--ip-color-bg-secondary); color: var(--ip-color-text-primary); }
</style>
