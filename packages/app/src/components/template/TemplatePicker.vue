<script setup lang="ts">
// Template 选择器
//
// 职责：
//   - 横向滚动的模板芯片行（chip row）
//   - 当前选中的模板高亮，再次点击可取消
//   - 点击非选中模板：若模板无变量 → 立即 apply；若模板有变量 → 打开变量填写弹窗
//   - 提供「@」自动补全 popover（在 ChatInput / WelcomeInput 中通过 expose 方法打开）
//   - 变量填写弹窗：每个变量对应一个输入控件（text/textarea/select）
//
// props:
//   - selectedId: 当前选中的模板 ID（受控）
//
// emits:
//   - update:selectedId [id]  用户切换选中（含取消：传 null）
//   - apply [payload]         用户确认应用模板，payload = { templateId, values }
//   - cancel-apply            用户取消变量填写弹窗

import { computed, ref, nextTick, watch } from "vue";
import { Modal, Button, Input, Textarea } from "@ice-paw/ui";
import { Sparkles, ChevronRight } from "lucide-vue-next";
import { useTemplatesStore } from "../../stores/templates";
import type { Template } from "../../types";

const props = defineProps<{
  selectedId: string | null;
}>();

const emit = defineEmits<{
  "update:selectedId": [value: string | null];
  apply: [payload: { templateId: string; values: Record<string, string> }];
  "cancel-apply": [];
}>();

const templatesStore = useTemplatesStore();

// ============================================================================
// 状态
// ============================================================================

/** 变量填写弹窗：当前正在填写的模板 */
const dialogTemplate = ref<Template | null>(null);

/** 变量填写弹窗：每个变量的当前值 */
const dialogValues = ref<Record<string, string>>({});

/** @ 自动补全 popover */
const autocompleteOpen = ref<boolean>(false);
const autocompleteQuery = ref<string>("");
const autocompleteAnchor = ref<{ x: number; y: number } | null>(null);
/** 当前 autocomplete 候选列表（index 选中） */
const autocompleteSelected = ref<number>(0);

/** autocomplete popover DOM 引用 */
const autocompleteRef = ref<HTMLDivElement | null>(null);

// ============================================================================
// 派生
// ============================================================================

const hasTemplates = computed<boolean>(() => templatesStore.templates.length > 0);

const autocompleteCandidates = computed<Template[]>(() => {
  const q = autocompleteQuery.value.trim().toLowerCase();
  const all = templatesStore.templates;
  if (!q) return all.slice(0, 8);
  return all
    .filter(
      (t) =>
        t.name.toLowerCase().includes(q) || t.description.toLowerCase().includes(q),
    )
    .slice(0, 8);
});

// ============================================================================
// 生命周期：挂载时确保模板已加载
// ============================================================================

import { onMounted } from "vue";

onMounted(() => {
  templatesStore.ensureLoaded();
});

// ============================================================================
// Chip 行交互
// ============================================================================

function onChipClick(template: Template): void {
  if (props.selectedId === template.id) {
    // 再次点击 → 取消选择
    emit("update:selectedId", null);
    return;
  }
  // 选中
  emit("update:selectedId", template.id);
  if (template.variables.length === 0) {
    // 无变量 → 立即 apply
    emit("apply", { templateId: template.id, values: {} });
  } else {
    // 有变量 → 打开填写弹窗
    openVariableDialog(template);
  }
}

/** 清除选择（外部触发时用，例如用户手动编辑了消息） */
function clearSelection(): void {
  emit("update:selectedId", null);
}

// ============================================================================
// 变量弹窗
// ============================================================================

function openVariableDialog(template: Template): void {
  dialogTemplate.value = template;
  // 预填默认值
  const init: Record<string, string> = {};
  for (const v of template.variables) {
    init[v.name] = v.default ?? "";
  }
  dialogValues.value = init;
}

function closeVariableDialog(): void {
  dialogTemplate.value = null;
  dialogValues.value = {};
  emit("cancel-apply");
}

function confirmVariableDialog(): void {
  if (!dialogTemplate.value) return;
  emit("apply", {
    templateId: dialogTemplate.value.id,
    values: { ...dialogValues.value },
  });
  dialogTemplate.value = null;
  dialogValues.value = {};
}

function getVariableValue(name: string): string {
  return dialogValues.value[name] ?? "";
}

function setVariableValue(name: string, value: string): void {
  dialogValues.value = { ...dialogValues.value, [name]: value };
}

// ============================================================================
// @ 自动补全
// ============================================================================

/**
 * 外部调用：触发 @ 自动补全
 *
 * @param anchor 光标坐标（textarea 内的 caret 位置）
 * @param query  触发时 @ 后的查询字符串
 */
function openAutocomplete(anchor: { x: number; y: number }, query: string): void {
  autocompleteAnchor.value = anchor;
  autocompleteQuery.value = query;
  autocompleteSelected.value = 0;
  autocompleteOpen.value = true;
}

/** 外部调用：关闭自动补全 */
function closeAutocomplete(): void {
  autocompleteOpen.value = false;
  autocompleteAnchor.value = null;
  autocompleteQuery.value = "";
}

/** 处理键盘：选择 / 应用 / 取消 */
function onAutocompleteKey(event: KeyboardEvent): boolean {
  if (!autocompleteOpen.value) return false;
  const candidates = autocompleteCandidates.value;
  if (candidates.length === 0) {
        if (event.key === "Escape") {
          closeAutocomplete();
          return true;
        }
        return false;
      }

  if (event.key === "ArrowDown") {
    autocompleteSelected.value = (autocompleteSelected.value + 1) % candidates.length;
    return true;
  }
  if (event.key === "ArrowUp") {
    autocompleteSelected.value =
      (autocompleteSelected.value - 1 + candidates.length) % candidates.length;
    return true;
  }
  if (event.key === "Enter" || event.key === "Tab") {
    const tpl = candidates[autocompleteSelected.value];
    if (tpl) {
      onChipClick(tpl);
      closeAutocomplete();
    }
    return true;
  }
  if (event.key === "Escape") {
    closeAutocomplete();
    return true;
  }
  return false;
}

function onAutocompleteItemClick(template: Template): void {
  onChipClick(template);
  closeAutocomplete();
}

// ============================================================================
// Expose 给父组件（ChatInput / WelcomeInput）
// ============================================================================

defineExpose({
  openAutocomplete,
  closeAutocomplete,
  onAutocompleteKey,
  clearSelection,
});

// ============================================================================
// 滚动到选中项（autocomplete 改变选中时）
// ============================================================================

watch(autocompleteSelected, () => {
  void nextTick(() => {
    const el = autocompleteRef.value?.querySelector(
      `.autocomplete-item[data-index="${autocompleteSelected.value}"]`,
    );
    if (el && "scrollIntoView" in el) {
      (el as HTMLElement).scrollIntoView({ block: "nearest" });
    }
  });
});
</script>

<template>
  <div class="template-picker">
    <!-- 模板芯片行 -->
    <div v-if="hasTemplates" class="chip-row" :aria-label="'模板芯片'">
      <div class="chip-row-inner">
        <button
          v-for="tpl in templatesStore.templates"
          :key="tpl.id"
          type="button"
          :class="[
            'chip',
            { 'chip-selected': props.selectedId === tpl.id },
          ]"
          :aria-pressed="props.selectedId === tpl.id"
          :title="tpl.description || tpl.name"
          @click="onChipClick(tpl)"
        >
          <Sparkles :size="12" />
          <span class="chip-name">{{ tpl.name }}</span>
          <span v-if="tpl.variables.length > 0" class="chip-var-count">
            {{ tpl.variables.length }}
          </span>
        </button>
      </div>
    </div>

    <!-- 选中状态指示器（无 chip row 时） -->
    <div v-if="!hasTemplates && templatesStore.loaded" class="empty-hint">
      还没有模板，
      <router-link to="/templates">前往创建</router-link>
    </div>

    <!-- @ 自动补全 popover -->
    <Teleport to="body">
      <div
        v-if="autocompleteOpen && autocompleteAnchor"
        ref="autocompleteRef"
        class="autocomplete-popover"
        :style="{
          left: `${autocompleteAnchor.x}px`,
          top: `${autocompleteAnchor.y}px`,
        }"
        role="listbox"
        @mousedown.prevent
      >
        <div v-if="autocompleteCandidates.length === 0" class="autocomplete-empty">
          没有匹配的模板
        </div>
        <div
          v-for="(tpl, idx) in autocompleteCandidates"
          :key="tpl.id"
          :class="[
            'autocomplete-item',
            { 'autocomplete-item-active': idx === autocompleteSelected },
          ]"
          :data-index="idx"
          role="option"
          :aria-selected="idx === autocompleteSelected"
          @click="onAutocompleteItemClick(tpl)"
          @mouseenter="autocompleteSelected = idx"
        >
          <Sparkles :size="14" />
          <div class="autocomplete-item-body">
            <div class="autocomplete-item-name">{{ tpl.name }}</div>
            <div v-if="tpl.description" class="autocomplete-item-desc">
              {{ tpl.description }}
            </div>
          </div>
          <ChevronRight :size="14" />
        </div>
      </div>
    </Teleport>

    <!-- 变量填写弹窗 -->
    <Modal
      :model-value="dialogTemplate !== null"
      size="md"
      :title="dialogTemplate ? `填写「${dialogTemplate.name}」变量` : ''"
      @update:model-value="(v) => { if (!v) closeVariableDialog() }"
    >
      <div v-if="dialogTemplate" class="variable-dialog-body">
        <p v-if="dialogTemplate.description" class="variable-dialog-desc">
          {{ dialogTemplate.description }}
        </p>

        <div
          v-for="v in dialogTemplate.variables"
          :key="v.name"
          class="variable-field"
        >
          <label class="variable-label" :for="`v-${v.name}`">
            {{ v.label }}
            <span v-if="v.default" class="variable-default">
              默认：{{ v.default }}
            </span>
          </label>

          <select
            v-if="v.type === 'select'"
            :id="`v-${v.name}`"
            class="variable-select"
            :value="getVariableValue(v.name)"
            @change="
              setVariableValue(
                v.name,
                ($event.target as HTMLSelectElement).value,
              )
            "
          >
            <option v-if="!v.default" value="" disabled>请选择</option>
            <option
              v-for="opt in v.options ?? []"
              :key="opt"
              :value="opt"
            >
              {{ opt }}
            </option>
          </select>

          <Textarea
            v-else-if="v.type === 'textarea'"
            :id="`v-${v.name}`"
            :rows="3"
            :model-value="getVariableValue(v.name)"
            @update:model-value="(val) => setVariableValue(v.name, val)"
          />

          <Input
            v-else
            :id="`v-${v.name}`"
            :model-value="getVariableValue(v.name)"
            @update:model-value="(val) => setVariableValue(v.name, val)"
          />
        </div>
      </div>

      <template #footer>
        <Button variant="secondary" @click="closeVariableDialog">取消</Button>
        <Button variant="primary" @click="confirmVariableDialog">应用</Button>
      </template>
    </Modal>
  </div>
</template>

<style scoped>
.template-picker {
  width: 100%;
}

.chip-row {
  width: 100%;
  overflow-x: auto;
  overflow-y: hidden;
  padding: var(--ip-spacing-2) 0;
  scrollbar-width: thin;
}

.chip-row-inner {
  display: inline-flex;
  gap: var(--ip-spacing-2);
  padding: 0 var(--ip-spacing-3);
}

.chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  height: 26px;
  padding: 0 var(--ip-spacing-3);
  background: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: 999px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  white-space: nowrap;
  flex-shrink: 0;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.chip:hover {
  background: var(--ip-color-bg-secondary);
  border-color: var(--ip-color-border-strong);
}

.chip-selected {
  background: var(--ip-primary-100, #dbeafe);
  border-color: var(--ip-primary-500, #3b82f6);
  color: var(--ip-primary-700, #1d4ed8);
}

.chip-name {
  font-weight: var(--ip-font-weight-medium);
}

.chip-var-count {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 16px;
  height: 16px;
  padding: 0 4px;
  background: var(--ip-color-bg-primary);
  border-radius: 999px;
  font-size: 10px;
  color: var(--ip-color-text-tertiary);
}

.chip-selected .chip-var-count {
  background: var(--ip-primary-200, #bfdbfe);
  color: var(--ip-primary-700, #1d4ed8);
}

.empty-hint {
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}
.empty-hint a {
  color: var(--ip-primary-600, #2563eb);
  text-decoration: none;
}

.autocomplete-popover {
  position: fixed;
  z-index: 1100;
  min-width: 240px;
  max-width: 360px;
  max-height: 240px;
  overflow-y: auto;
  background: var(--ip-color-bg-primary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-md, 0 8px 24px rgba(0, 0, 0, 0.12));
  padding: 4px;
}

.autocomplete-empty {
  padding: var(--ip-spacing-3);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  text-align: center;
}

.autocomplete-item {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-2);
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  color: var(--ip-color-text-secondary);
  transition: background var(--ip-duration-fast) var(--ip-ease-out);
}

.autocomplete-item-active {
  background: var(--ip-color-bg-secondary);
  color: var(--ip-color-text-primary);
}

.autocomplete-item-body {
  flex: 1;
  min-width: 0;
}

.autocomplete-item-name {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.autocomplete-item-desc {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.variable-dialog-body {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-4);
}

.variable-dialog-desc {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-tertiary);
}

.variable-field {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
}

.variable-label {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
}

.variable-default {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  font-weight: var(--ip-font-weight-regular);
}

.variable-select {
  height: 36px;
  padding: 0 var(--ip-spacing-3);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  background: var(--ip-color-bg-primary);
  color: var(--ip-color-text-primary);
  font-size: var(--ip-text-body-sm-size);
  font-family: inherit;
  cursor: pointer;
}
.variable-select:focus {
  outline: none;
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}
</style>
