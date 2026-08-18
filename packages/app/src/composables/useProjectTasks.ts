// useProjectTasks.ts — 项目任务台账数据源（MA-2）。
// load 拉静态行；live 更新事件驱动 + 去抖 300ms，不做常驻轮询：
// - chat:delegation-started：新委派子会话落地 → refresh（新任务入账）
// - session:event-appended(kind=turn_ended)：任务终态翻转的唯一信号
//   （流式 chunk 等噪声被 kind 过滤；只认任务集内的会话）
// onActivated 补拉由页面层调 refresh（keep-alive 离开期间错过的事件兜底）。
import { ref, onMounted, onBeforeUnmount, toValue } from "vue";
import type { MaybeRefOrGetter } from "vue";
import { listen } from "@tauri-apps/api/event";
import { bridge } from "../api/bridge";
import type { ProjectTask } from "../types";

export function useProjectTasks(projectId: MaybeRefOrGetter<string>) {
  const tasks = ref<ProjectTask[]>([]);
  const loading = ref(false);
  const error = ref("");

  async function load() {
    const pid = toValue(projectId);
    if (!pid) return;
    loading.value = true;
    error.value = "";
    try {
      tasks.value = await bridge.projects.listTasks(pid);
    } catch (e) {
      error.value = e instanceof Error ? e.message : "加载任务台账失败";
    } finally {
      loading.value = false;
    }
  }

  /** live 去抖：同一秒内多任务相继 turn_ended 只拉一次 */
  let refreshTimer: ReturnType<typeof setTimeout> | null = null;
  function refresh() {
    if (refreshTimer) clearTimeout(refreshTimer);
    refreshTimer = setTimeout(() => {
      refreshTimer = null;
      void load();
    }, 300);
  }

  const unlisteners: Array<() => void> = [];
  onMounted(async () => {
    await load();
    unlisteners.push(
      // 新任务入账（child_conversation_id 落地即达，不必等 tool_result 回传）。
      // payload 只有父会话/子会话/agent/title，无项目字段——无法精确过滤，
      // 任何委派都 refresh：单查询代价 + 去抖兜住，宁可多刷不可漏刷
      await listen("chat:delegation-started", refresh),
    );
    unlisteners.push(
      await listen<{ conversation_id: string; kind: string }>(
        "session:event-appended",
        (e) => {
          if (e.payload.kind !== "turn_ended") return;
          // 只认任务集内的会话（turn_ended 高频——任何会话每轮都发）
          const ids = new Set(tasks.value.map((t) => t.conv_id));
          if (!ids.has(e.payload.conversation_id)) return;
          refresh();
        },
      ),
    );
  });
  onBeforeUnmount(() => {
    unlisteners.forEach((u) => u());
    unlisteners.length = 0;
    if (refreshTimer) clearTimeout(refreshTimer);
  });

  return { tasks, loading, error, load, refresh };
}
