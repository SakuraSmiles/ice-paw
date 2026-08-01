// composables/useNewConversation.ts
// 「新建会话」统一逻辑（侧栏「新建对话」+ 欢迎页 CTA 共用），
// 保证两处对项目成员的限制完全一致，不再「这边限、那边漏」。
//
// 规则：
// - 项目内有成员 → 选择器只列成员（仅 1 个则直接建，跳过选择）
// - 项目内无成员 → 引导「去添加成员」（不弹全量选择器）
// - 散落空间 → 全量 agent
// - 全局无 agent → 引导「去创建智能体」

import { ref, computed } from "vue";
import { useRouter } from "vue-router";
import { useProjectStore } from "../stores/project";
import { useAgentStore } from "../stores/agent";
import { useChatStore } from "../stores/chat";

export function useNewConversation() {
  const project = useProjectStore();
  const agent = useAgentStore();
  const chat = useChatStore();
  const router = useRouter();

  const showPicker = ref(false);

  const inProject = computed(() => project.activeProjectId !== null);
  const memberAgentIds = computed(() =>
    (project.activeProject?.agents ?? []).map((a) => a.agent_id),
  );
  const hasMembers = computed(() => memberAgentIds.value.length > 0);
  const hasAgents = computed(() => agent.list.length > 0);

  /** 选择器范围：项目内→仅成员；散落→全部（undefined）；项目无成员不会走到选择器 */
  const pickerAgentIds = computed<string[] | undefined>(() =>
    inProject.value ? memberAgentIds.value : undefined,
  );

  /** 入口行为：无 agent / 项目无成员 / 正常新建 */
  const ctaKind = computed<"no-agents" | "no-members" | "new-chat">(() => {
    if (!hasAgents.value) return "no-agents";
    if (inProject.value && !hasMembers.value) return "no-members";
    return "new-chat";
  });
  const ctaLabel = computed(() => {
    if (ctaKind.value === "no-agents") return "去创建智能体";
    if (ctaKind.value === "no-members") return "去添加成员";
    return "新建对话";
  });

  /** 真正建会话：归入当前项目空间，并回到首页展示。 */
  async function create(agentId: string) {
    showPicker.value = false;
    try {
      await chat.createConversation(agentId, project.activeProjectId);
      if (router.currentRoute.value.name !== "Home") router.push("/");
    } catch (e) {
      console.error("新建会话失败:", e);
    }
  }

  /** 入口点击：按 ctaKind 决定跳转 / 直接建 / 弹选择器。 */
  function startNew() {
    if (ctaKind.value === "no-agents") { router.push("/settings/agents"); return; }
    if (ctaKind.value === "no-members") { router.push("/projects"); return; }
    const ids = pickerAgentIds.value;
    const count = ids ? ids.length : agent.list.length;
    if (count === 1) {
      create(ids ? ids[0] : agent.list[0].id);
      return;
    }
    showPicker.value = true;
  }

  function onPickAgent(agentId: string) {
    create(agentId);
  }

  return { showPicker, pickerAgentIds, ctaKind, ctaLabel, startNew, onPickAgent };
}
