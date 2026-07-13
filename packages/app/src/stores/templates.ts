// IcePaw 模板状态管理 Store
//
// 职责：
//   1. 维护前端可用的 Template 列表
//   2. 维护当前选中的模板（聊天中@ 触发时设置）
//   3. 提供 CRUD 操作的 actions，封装 bridge.templates.* 调用与本地状态同步
//
// 设计要点：
//   - Composition API 风格（与 stores/agents.ts / stores/chat.ts 一致）
//   - selectedId 持久化到 localStorage（下次启动自动恢复）
//   - 聊天中模板选择需要「已填写的变量值」，本 store 仅保存选中 ID，
//     实际值由 ChatInput / WelcomeInput 临时维护（一次性的会话内状态）。
//
// 持久化：
//   - selectedId 持久化到 localStorage('icepaw.lastTemplate')
//   - loaded 标记持久化到 localStorage('icepaw.templatesLoaded')

import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { bridge } from "../api/bridge";
import type { NewTemplate, Template, TemplateUpdate } from "../types";

const LS_LAST_TEMPLATE = "icepaw.lastTemplate";
const LS_LOADED_FLAG = "icepaw.templatesLoaded";

/**
 * Template Store
 *
 * state:
 *   - templates   模板列表
 *   - selectedId  当前选中的模板 ID（用于 @ 触发；null 表示未选）
 *   - loading     加载中
 *   - error       最近错误信息
 *   - loaded      是否已成功加载
 *
 * getters:
 *   - selected    当前选中的模板实体
 *   - byId(id)    按 ID 查找
 *   - byName(name) 按 name 查找（@ 自动补全用）
 *
 * actions:
 *   - ensureLoaded()   幂等加载
 *   - fetchAll()       强制刷新
 *   - setSelected(id)  设置当前选中（持久化，传 null 清空）
 *   - createOne(input) 创建
 *   - updateOne(id, patch) 更新
 *   - deleteOne(id)    删除
 */
export const useTemplatesStore = defineStore("templates", () => {
  // ============================================================================
  // state
  // ============================================================================

  const templates = ref<Template[]>([]);
  const selectedId = ref<string | null>(null);
  const loading = ref<boolean>(false);
  const error = ref<string | null>(null);
  const loaded = ref<boolean>(false);

  // ============================================================================
  // getters
  // ============================================================================

  const selected = computed<Template | null>(() => {
    if (!selectedId.value) return null;
    return templates.value.find((t) => t.id === selectedId.value) ?? null;
  });

  function byId(id: string): Template | undefined {
    return templates.value.find((t) => t.id === id);
  }

  function byName(name: string): Template | undefined {
    const lower = name.toLowerCase();
    return templates.value.find((t) => t.name.toLowerCase() === lower);
  }

  // ============================================================================
  // actions
  // ============================================================================

  async function ensureLoaded(): Promise<void> {
    if (loaded.value) return;

    // 同步 localStorage 标记
    try {
      if (localStorage.getItem(LS_LOADED_FLAG) === "1") {
        loaded.value = true;
      }
      if (loaded.value) {
        await fetchAll();
        restoreSelected();
        return;
      }
    } catch {
      // localStorage 不可用时忽略
    }

    await fetchAll();

    try {
      localStorage.setItem(LS_LOADED_FLAG, "1");
      loaded.value = true;
    } catch {
      // 忽略
    }
    restoreSelected();
  }

  function restoreSelected(): void {
    try {
      const saved = localStorage.getItem(LS_LAST_TEMPLATE);
      if (saved && templates.value.some((t) => t.id === saved)) {
        selectedId.value = saved;
      } else {
        selectedId.value = null;
      }
    } catch {
      selectedId.value = null;
    }
  }

  async function fetchAll(): Promise<void> {
    loading.value = true;
    error.value = null;
    try {
      templates.value = await bridge.templates.list();
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    } finally {
      loading.value = false;
    }
  }

  function setSelected(id: string | null): void {
    if (id !== null && !templates.value.some((t) => t.id === id)) {
      return;
    }
    selectedId.value = id;
    try {
      if (id === null) {
        localStorage.removeItem(LS_LAST_TEMPLATE);
      } else {
        localStorage.setItem(LS_LAST_TEMPLATE, id);
      }
    } catch {
      // 忽略
    }
  }

  async function createOne(input: NewTemplate): Promise<Template> {
    error.value = null;
    try {
      const created = await bridge.templates.create(input);
      templates.value = [...templates.value, created].sort(
        (a, b) => a.sort_order - b.sort_order || a.created_at.localeCompare(b.created_at),
      );
      return created;
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    }
  }

  async function updateOne(id: string, patch: TemplateUpdate): Promise<Template> {
    error.value = null;
    try {
      const updated = await bridge.templates.update({ ...patch, id });
      const idx = templates.value.findIndex((t) => t.id === id);
      if (idx >= 0) {
        templates.value.splice(idx, 1, updated);
      } else {
        templates.value.push(updated);
      }
      templates.value = [...templates.value].sort(
        (a, b) => a.sort_order - b.sort_order || a.created_at.localeCompare(b.created_at),
      );
      return updated;
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    }
  }

  async function deleteOne(id: string): Promise<void> {
    error.value = null;
    try {
      await bridge.templates.delete(id);
      const idx = templates.value.findIndex((t) => t.id === id);
      if (idx >= 0) {
        templates.value.splice(idx, 1);
      }
      if (selectedId.value === id) {
        setSelected(null);
      }
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    }
  }

  function clearError(): void {
    error.value = null;
  }

  return {
    // state
    templates,
    selectedId,
    loading,
    error,
    loaded,
    // getters
    selected,
    byId,
    byName,
    // actions
    ensureLoaded,
    fetchAll,
    setSelected,
    createOne,
    updateOne,
    deleteOne,
    clearError,
  };
});
