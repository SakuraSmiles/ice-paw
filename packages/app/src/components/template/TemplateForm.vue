<script setup lang="ts">
// Template 创建/编辑表单 — 侧滑面板
//
// 职责：
//   - create 模式：空表单 → 填完提交
//   - edit   模式：回填现有 Template → 改完提交（partial update）
//   - 字段：name / description / system_prompt / user_prompt_prefix
//          / variables[]（动态增删）/ tools[]（逗号分隔文本）
//          / sort_order
//
// props:
//   - mode:   "create" | "edit"
//   - template: 编辑模式下的目标 Template
//   - open:   面板是否显示
//
// emits:
//   - update:open  关闭面板
//   - submit       提交表单，payload 含完整 NewTemplate / TemplateUpdate 字段

import { ref, computed, watch } from "vue";
import { Input, Textarea, Button } from "@ice-paw/ui";
import { X, Plus, Trash2 } from "lucide-vue-next";
import type { Template, TemplateVariable } from "../../types";

const VARIABLE_TYPES = ["text", "textarea", "select"] as const;
type VariableType = (typeof VARIABLE_TYPES)[number];

/**
 * 提交 payload。
 * create 模式：传完整字段（NewTemplate 风格）
 * edit 模式：仅传改过的字段（TemplateUpdate 风格）
 *
 * 这里统一为「带 mode 标记的 payload」，由父组件按 mode 分发到
 * createOne / updateOne。
 */
export interface TemplateFormPayload {
  mode: "create" | "edit";
  /** create 模式：完整新模板 */
  newTemplate?: {
    name: string;
    description: string;
    system_prompt: string;
    user_prompt_prefix: string;
    variables: TemplateVariable[];
    tools: string[];
    sort_order: number;
  };
  /** edit 模式：仅改过的字段（partial update） */
  patch?: {
    name?: string;
    description?: string;
    system_prompt?: string;
    user_prompt_prefix?: string;
    variables?: TemplateVariable[];
    tools?: string[];
    sort_order?: number;
  };
}

const props = defineProps<{
  mode: "create" | "edit";
  template: Template | null;
  open: boolean;
}>();

const emit = defineEmits<{
  "update:open": [value: boolean];
  submit: [payload: TemplateFormPayload];
}>();

// ============================================================================
// 表单状态
// ============================================================================

const name = ref<string>("");
const description = ref<string>("");
const systemPrompt = ref<string>("");
const userPromptPrefix = ref<string>("");
const variables = ref<TemplateVariable[]>([]);
const toolsText = ref<string>("");
const sortOrder = ref<number>(0);

const errors = ref<Record<string, string>>({});

const nameError = computed<string>(() => errors.value.name ?? "");

// ============================================================================
// 联动：面板打开时重置 / 回填
// ============================================================================

watch(
  () => props.open,
  (val) => {
    if (!val) return;
    resetForm();
    if (props.mode === "edit" && props.template) {
      populateFromTemplate(props.template);
    }
  },
);

function resetForm(): void {
  name.value = "";
  description.value = "";
  systemPrompt.value = "";
  userPromptPrefix.value = "";
  variables.value = [];
  toolsText.value = "";
  sortOrder.value = 0;
  errors.value = {};
}

function populateFromTemplate(tpl: Template): void {
  name.value = tpl.name;
  description.value = tpl.description;
  systemPrompt.value = tpl.system_prompt;
  userPromptPrefix.value = tpl.user_prompt_prefix;
  variables.value = JSON.parse(JSON.stringify(tpl.variables));
  toolsText.value = tpl.tools.join(", ");
  sortOrder.value = tpl.sort_order;
  errors.value = {};
}

// ============================================================================
// 变量增删
// ============================================================================

function addVariable(): void {
  variables.value = [
    ...variables.value,
    {
      name: "",
      label: "",
      type: "text",
      default: null,
      options: null,
    },
  ];
}

function removeVariable(idx: number): void {
  variables.value = variables.value.filter((_, i) => i !== idx);
}

function setVariableType(idx: number, type: VariableType): void {
  const next = [...variables.value];
  const cur = next[idx];
  if (!cur) return;
  next[idx] = { ...cur, type };
  // select 类型：保证 options 是数组
  if (type === "select" && !cur.options) {
    next[idx] = { ...next[idx], options: [] };
  }
  variables.value = next;
}

function setVariableOptionsText(idx: number, text: string): void {
  const next = [...variables.value];
  const cur = next[idx];
  if (!cur) return;
  const list = text
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  next[idx] = { ...cur, options: list };
  variables.value = next;
}

// ============================================================================
// 校验
// ============================================================================

function validate(): boolean {
  const e: Record<string, string> = {};
  if (!name.value.trim()) {
    e.name = "名称不能为空";
  }
  // 变量名去重 + 命名合法性
  const names = new Set<string>();
  for (const v of variables.value) {
    const n = v.name.trim();
    if (!n) {
      // 允许临时空名（用户正在编辑），但提交时不允许
      e.variables = "变量名不能为空";
      break;
    }
    if (!/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(n)) {
      e.variables = `变量名「${n}」不合法（需以字母/下划线开头，仅含字母数字下划线）`;
      break;
    }
    if (names.has(n)) {
      e.variables = `变量名「${n}」重复`;
      break;
    }
    names.add(n);
    if (!v.label.trim()) {
      e.variables = `变量「${n}」的标签不能为空`;
      break;
    }
    if (v.type === "select" && (!v.options || v.options.length === 0)) {
      e.variables = `select 变量「${n}」必须至少有一个选项`;
      break;
    }
  }
  errors.value = e;
  return Object.keys(e).length === 0;
}

// ============================================================================
// 提交
// ============================================================================

function buildVariablesFromState(): TemplateVariable[] {
  return variables.value.map((v) => {
    const out: TemplateVariable = {
      name: v.name.trim(),
      label: v.label.trim(),
      type: v.type,
      default: v.default && v.default.length > 0 ? v.default : null,
    };
    if (v.type === "select") {
      out.options = v.options ?? [];
    }
    return out;
  });
}

function buildToolsFromText(): string[] {
  return toolsText.value
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

function handleSubmit(): void {
  if (!validate()) return;

  const vars = buildVariablesFromState();
  const tools = buildToolsFromText();

  if (props.mode === "create") {
    emit("submit", {
      mode: "create",
      newTemplate: {
        name: name.value.trim(),
        description: description.value.trim(),
        system_prompt: systemPrompt.value,
        user_prompt_prefix: userPromptPrefix.value,
        variables: vars,
        tools,
        sort_order: sortOrder.value,
      },
    });
  } else {
    // edit：仅传与原值不同的字段
    const tpl = props.template;
    if (!tpl) return;
    const patch: TemplateFormPayload["patch"] = {};
    if (name.value.trim() !== tpl.name) patch.name = name.value.trim();
    if (description.value.trim() !== tpl.description)
      patch.description = description.value.trim();
    if (systemPrompt.value !== tpl.system_prompt)
      patch.system_prompt = systemPrompt.value;
    if (userPromptPrefix.value !== tpl.user_prompt_prefix)
      patch.user_prompt_prefix = userPromptPrefix.value;
    if (JSON.stringify(vars) !== JSON.stringify(tpl.variables)) patch.variables = vars;
    if (JSON.stringify(tools) !== JSON.stringify(tpl.tools)) patch.tools = tools;
    if (sortOrder.value !== tpl.sort_order) patch.sort_order = sortOrder.value;
    emit("submit", { mode: "edit", patch });
  }
}

function handleClose(): void {
  emit("update:open", false);
}

const panelTitle = computed<string>(() =>
  props.mode === "create" ? "新建模板" : "编辑模板",
);

// 渲染用：select 类型变量的 options 文本
function getOptionsText(v: TemplateVariable): string {
  return v.options?.join(", ") ?? "";
}
</script>

<template>
  <Teleport to="body">
    <div v-if="props.open" class="tpl-form-overlay" @click.self="handleClose">
      <div class="tpl-form-panel" role="dialog" aria-modal="true" :aria-label="panelTitle">
        <header class="tpl-form-header">
          <h2 class="tpl-form-title">{{ panelTitle }}</h2>
          <button
            type="button"
            class="tpl-form-close"
            aria-label="关闭"
            @click="handleClose"
          >
            <X :size="18" />
          </button>
        </header>

        <div class="tpl-form-body">
          <!-- 名称 -->
          <div class="field">
            <label class="field-label" for="tpl-name">名称 *</label>
            <Input
              id="tpl-name"
              v-model="name"
              placeholder="如：代码评审"
              :error="!!nameError"
              :error-message="nameError"
            />
          </div>

          <!-- 描述 -->
          <div class="field">
            <label class="field-label" for="tpl-desc">描述</label>
            <Input
              id="tpl-desc"
              v-model="description"
              placeholder="一句话说明模板用途（列表展示用）"
            />
          </div>

          <!-- System Prompt -->
          <div class="field">
            <label class="field-label" for="tpl-sys">System Prompt</label>
            <Textarea
              id="tpl-sys"
              v-model="systemPrompt"
              :rows="5"
              placeholder="可使用 {{变量名}} 引用变量"
            />
            <p class="field-hint">
              可用 <code>&#123;&#123;变量名&#125;&#125;</code> 占位变量，发送时由用户填入
            </p>
          </div>

          <!-- User Prompt Prefix -->
          <div class="field">
            <label class="field-label" for="tpl-usr">User Prompt 前缀</label>
            <Textarea
              id="tpl-usr"
              v-model="userPromptPrefix"
              :rows="3"
              placeholder="如：请评审以下 {{language}} 代码：\n"
            />
            <p class="field-hint">拼到用户消息前面</p>
          </div>

          <!-- 变量列表 -->
          <div class="field">
            <div class="field-label-row">
              <span class="field-label">变量</span>
              <Button variant="ghost" size="sm" @click="addVariable">
                <template #icon-left>
                  <Plus :size="14" />
                </template>
                添加
              </Button>
            </div>

            <div v-if="variables.length === 0" class="variables-empty">
              暂无变量
            </div>

            <div v-else class="variables-list">
              <div
                v-for="(v, idx) in variables"
                :key="idx"
                class="variable-row"
              >
                <div class="variable-row-grid">
                  <Input
                    v-model="v.name"
                    placeholder="name（英文）"
                    size="sm"
                  />
                  <Input
                    v-model="v.label"
                    placeholder="标签（中文）"
                    size="sm"
                  />
                  <select
                    class="variable-type-select"
                    :value="v.type"
                    aria-label="变量类型"
                    @change="
                      setVariableType(
                        idx,
                        ($event.target as HTMLSelectElement).value as VariableType,
                      )
                    "
                  >
                    <option v-for="t in VARIABLE_TYPES" :key="t" :value="t">
                      {{ t }}
                    </option>
                  </select>
                  <Input
                    v-model="v.default"
                    placeholder="默认值（可选）"
                    size="sm"
                  />
                </div>

                <div v-if="v.type === 'select'" class="variable-row-options">
                  <Input
                    :model-value="getOptionsText(v)"
                    placeholder="选项，逗号分隔"
                    size="sm"
                    @update:model-value="(val) => setVariableOptionsText(idx, val)"
                  />
                </div>

                <button
                  type="button"
                  class="variable-remove"
                  :aria-label="`删除变量 ${v.name || idx}`"
                  @click="removeVariable(idx)"
                >
                  <Trash2 :size="14" />
                </button>
              </div>
            </div>

            <p v-if="errors.variables" class="field-error">
              {{ errors.variables }}
            </p>
          </div>

          <!-- 工具列表 -->
          <div class="field">
            <label class="field-label" for="tpl-tools">工具（逗号分隔）</label>
            <Input
              id="tpl-tools"
              v-model="toolsText"
              placeholder="read_file, shell_command"
            />
            <p class="field-hint">
              P2-1 工具调用落地后生效；当前仅记录不执行
            </p>
          </div>

          <!-- 排序权重 -->
          <div class="field">
            <label class="field-label" for="tpl-sort">排序权重</label>
            <Input
              id="tpl-sort"
              v-model.number="sortOrder"
              type="number"
              placeholder="0"
            />
            <p class="field-hint">值小者靠前</p>
          </div>
        </div>

        <footer class="tpl-form-footer">
          <Button variant="secondary" @click="handleClose">取消</Button>
          <Button variant="primary" @click="handleSubmit">
            {{ props.mode === "create" ? "创建" : "保存" }}
          </Button>
        </footer>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.tpl-form-overlay {
  position: fixed;
  inset: 0;
  z-index: 1000;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  justify-content: flex-end;
  animation: fade-in var(--ip-duration-fast) var(--ip-ease-out);
}

.tpl-form-panel {
  display: flex;
  flex-direction: column;
  width: min(640px, 100vw);
  height: 100vh;
  background: var(--ip-color-bg-primary);
  box-shadow: -8px 0 24px rgba(0, 0, 0, 0.15);
  animation: slide-in var(--ip-duration-base) var(--ip-ease-out);
}

.tpl-form-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--ip-spacing-4) var(--ip-spacing-5);
  border-bottom: 1px solid var(--ip-color-border-default);
}

.tpl-form-title {
  margin: 0;
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.tpl-form-close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--ip-radius-md);
  background: transparent;
  border: 0;
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  transition: background var(--ip-duration-fast) var(--ip-ease-out);
}
.tpl-form-close:hover {
  background: var(--ip-color-bg-secondary);
}

.tpl-form-body {
  flex: 1;
  overflow-y: auto;
  padding: var(--ip-spacing-5);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-4);
}

.tpl-form-footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--ip-spacing-3);
  padding: var(--ip-spacing-4) var(--ip-spacing-5);
  border-top: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-secondary);
}

.field {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}

.field-label-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.field-label {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
}

.field-hint {
  margin: 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}
.field-hint code {
  background: var(--ip-color-bg-tertiary);
  padding: 1px 4px;
  border-radius: var(--ip-radius-sm);
  font-family: monospace;
}

.field-error {
  margin: 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-error);
}

.variables-empty {
  padding: var(--ip-spacing-3);
  text-align: center;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
  background: var(--ip-color-bg-secondary);
  border: 1px dashed var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
}

.variables-list {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}

.variable-row {
  display: flex;
  align-items: flex-start;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-2);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
}

.variable-row-grid {
  flex: 1;
  display: grid;
  grid-template-columns: 1.2fr 1.2fr 0.8fr 1fr;
  gap: var(--ip-spacing-2);
}

.variable-row-options {
  flex: 1;
  margin-top: var(--ip-spacing-2);
}

.variable-type-select {
  height: 32px;
  padding: 0 var(--ip-spacing-2);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  background: var(--ip-color-bg-primary);
  color: var(--ip-color-text-primary);
  font-size: var(--ip-text-body-sm-size);
  cursor: pointer;
}

.variable-remove {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  border-radius: var(--ip-radius-sm);
  background: transparent;
  border: 0;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  flex-shrink: 0;
  margin-top: 2px;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.variable-remove:hover {
  background: var(--ip-color-error-bg, #fee);
  color: var(--ip-color-error);
}

@keyframes fade-in {
  from { opacity: 0; }
  to { opacity: 1; }
}

@keyframes slide-in {
  from { transform: translateX(100%); }
  to { transform: translateX(0); }
}
</style>
