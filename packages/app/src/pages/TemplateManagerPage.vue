<script setup lang="ts">
// 模板管理页
//
// 职责：
//   - 顶部「+ 新建模板」按钮
//   - 模板卡片列表
//   - 无数据时显示 EmptyTemplateHint
//   - 挂载 TemplateForm（侧滑面板）+ ConfirmDeleteDialog（Modal）
//   - 处理创建/编辑/删除的全部业务逻辑

import { ref, onMounted } from "vue";
import { Button } from "@ice-paw/ui";
import { Plus } from "lucide-vue-next";
import { useTemplatesStore } from "../stores/templates";
import { useToast } from "../composables/useToast";
import type { Template } from "../types";
import type { TemplateFormPayload } from "../components/template/TemplateForm.vue";
import TemplateList from "../components/template/TemplateList.vue";
import TemplateForm from "../components/template/TemplateForm.vue";
import ConfirmDeleteDialog from "../components/agent/ConfirmDeleteDialog.vue";
import EmptyTemplateHint from "../components/template/EmptyTemplateHint.vue";

const templatesStore = useTemplatesStore();
const toast = useToast();

// ============================================================================
// 表单状态
// ============================================================================

const formOpen = ref(false);
const formMode = ref<"create" | "edit">("create");
const editingTemplate = ref<Template | null>(null);

// ============================================================================
// 删除确认弹窗状态
// ============================================================================

const deleteDialogOpen = ref(false);
const deleteTarget = ref<Template | null>(null);

// ============================================================================
// 生命周期
// ============================================================================

onMounted(() => {
  templatesStore.ensureLoaded();
});

// ============================================================================
// 创建
// ============================================================================

function openCreate(): void {
  formMode.value = "create";
  editingTemplate.value = null;
  formOpen.value = true;
}

// ============================================================================
// 编辑
// ============================================================================

function openEdit(template: Template): void {
  formMode.value = "edit";
  editingTemplate.value = template;
  formOpen.value = true;
}

// ============================================================================
// 删除
// ============================================================================

function requestDelete(template: Template): void {
  deleteTarget.value = template;
  deleteDialogOpen.value = true;
}

async function confirmDelete(): Promise<void> {
  if (!deleteTarget.value) return;
  const id = deleteTarget.value.id;
  deleteDialogOpen.value = false;
  try {
    await templatesStore.deleteOne(id);
    toast.success("已删除");
  } catch {
    toast.error("删除失败");
  }
  deleteTarget.value = null;
}

function onDeleteDialogClose(): void {
  deleteDialogOpen.value = false;
  deleteTarget.value = null;
}

// ============================================================================
// 表单提交
// ============================================================================

async function handleFormSubmit(payload: TemplateFormPayload): Promise<void> {
  try {
    if (payload.mode === "create" && payload.newTemplate) {
      await templatesStore.createOne(payload.newTemplate);
      toast.success("模板已创建");
    } else if (payload.mode === "edit" && payload.patch) {
      await templatesStore.updateOne(editingTemplate.value!.id, {
        id: editingTemplate.value!.id,
        ...payload.patch,
      });
      toast.success("模板已更新");
    }
    formOpen.value = false;
  } catch {
    toast.error("保存失败");
  }
}
</script>

<template>
  <div class="template-manager">
    <!-- 页头 -->
    <div class="page-header">
      <h1 class="page-title">模板管理</h1>
      <Button variant="primary" size="md" @click="openCreate">
        <template #icon-left>
          <Plus :size="16" aria-hidden="true" />
        </template>
        新建模板
      </Button>
    </div>

    <!-- 加载中 -->
    <div v-if="templatesStore.loading && templatesStore.templates.length === 0" class="loading-hint">
      <span>加载中...</span>
    </div>

    <!-- 列表 / 空状态 -->
    <template v-else-if="templatesStore.templates.length > 0">
      <div class="template-list">
        <TemplateList
          v-for="tpl in templatesStore.templates"
          :key="tpl.id"
          :template="tpl"
          @edit="openEdit"
          @delete="requestDelete"
        />
      </div>
    </template>
    <template v-else>
      <EmptyTemplateHint @create="openCreate" />
    </template>
  </div>

  <!-- 创建/编辑表单面板 -->
  <TemplateForm
    :mode="formMode"
    :template="editingTemplate"
    :open="formOpen"
    @update:open="formOpen = $event"
    @submit="handleFormSubmit"
  />

  <!-- 删除确认弹窗 -->
  <ConfirmDeleteDialog
    v-model:open="deleteDialogOpen"
    title="确认删除"
    :message="`将删除模板「${deleteTarget?.name ?? ''}」，此操作不可撤销。`"
    @confirm="confirmDelete"
    @update:open="onDeleteDialogClose"
  />
</template>

<style scoped>
.template-manager {
  max-width: 720px;
  margin: 0 auto;
  padding: var(--ip-spacing-8) var(--ip-spacing-6);
}

.page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-4);
  margin-bottom: var(--ip-spacing-6);
}

.page-title {
  margin: 0;
  font-size: var(--ip-text-h2-size);
  font-weight: var(--ip-font-weight-semibold);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-primary);
}

.template-list {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
}

.loading-hint {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: var(--ip-spacing-12);
  color: var(--ip-color-text-tertiary);
}
</style>
