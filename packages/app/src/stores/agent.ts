// Agent 状态管理
import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type { Agent } from "../types";
import { bridge } from "../api/bridge";

export const useAgentStore = defineStore("agent", () => {
  const list = ref<Agent[]>([]);
  const loading = ref(false);
  const loaded = ref(false);

  async function load(force = false) {
    if (loaded.value && !force) return;
    loading.value = true;
    try {
      list.value = await bridge.agents.list();
      loaded.value = true;
    } catch (e) {
      console.error("加载 Agent 列表失败:", e);
    } finally {
      loading.value = false;
    }
  }

  const firstAgent = computed(() => list.value[0] ?? null);
  const getById = (id: string) => list.value.find((a) => a.id === id) ?? null;

  return { list, loading, loaded, load, firstAgent, getById };
});
