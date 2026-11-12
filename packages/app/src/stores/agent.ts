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

  /** 用户可见 agent（Router 化批 P1-1 收口）：过滤 ephemeral 临时 worker 与
   *  router 系统实体——**一切列表/选择器/默认位消费一律走这里**，勿直接读 list
   * （list 保留全量：getById 名字解析允许命中 worker，委派卡/台账要能显示名字）。 */
  const visibleAgents = computed(() =>
    list.value.filter((a) => !a.agent_kind || a.agent_kind === "standard"),
  );
  /** 首个可用 agent（默认位语义）——基于 visibleAgents，ephemeral/router 永不入选 */
  const firstAgent = computed(() => visibleAgents.value[0] ?? null);
  const getById = (id: string) => list.value.find((a) => a.id === id) ?? null;

  return { list, loading, loaded, load, firstAgent, visibleAgents, getById };
});
