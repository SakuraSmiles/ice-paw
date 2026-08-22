// AgentPicker.avatar.test.ts — 头像接线锁：列表项渲染 EntityAvatar
// （用户图 → 默认头像图，2026-08-22 拍板——智能体列表语境无图走默认图），
// 不再回退到手写首字母 + 硬编码蓝渐变（picker-avatar 文本节点即回归信号）。
import { describe, it, expect, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import AgentPicker from "../AgentPicker.vue";
import { useAgentStore } from "../../../stores/agent";
import type { Agent } from "../../../types";

function agent(overrides?: Partial<Agent>): Agent {
  return {
    id: "ag-1",
    name: "代码助手",
    provider: "openai",
    model: "gpt-4o",
    system_prompt: "",
    base_url: null,
    temperature: 0.7,
    max_tokens: 16384,
    extra_params: {},
    sort_order: 0,
    cache_prompt: true,
    workspace_path: null,
    config_from_file: false,
    created_at: "2026-08-15 00:00:00",
    updated_at: "2026-08-15 00:00:00",
    has_api_key: true,
    ...overrides,
  } as Agent;
}

function mountPicker() {
  const store = useAgentStore();
  store.list = [
    agent({ id: "a-img", name: "图片位", avatar: "data:image/webp;base64,xxx" }),
    agent({ id: "a-fallback", name: "兜底位" }),
  ];
  return mount(AgentPicker);
}

describe("AgentPicker 头像接线", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it("每行渲染 EntityAvatar：用户图出 <img>、无图走默认头像图", () => {
    const w = mountPicker();
    const avatars = w.findAllComponents({ name: "EntityAvatar" });
    expect(avatars.length).toBe(2);
    // 图片档：img src 指向 dataURL
    expect(avatars[0].find("img").attributes("src")).toBe("data:image/webp;base64,xxx");
    // 无图档：默认头像图（智能体列表语境，非渐变首字）
    expect(avatars[1].find("img").attributes("src")).toContain("default-agent-avatar");
    // 旧手写实现回归信号：picker-avatar 里不应再有裸文本节点渲染链
    expect(w.find(".picker-avatar").classes()).toContain("entity-avatar");
  });

  it("agentIds 限定候选集（项目成员过滤不受头像接线影响）", () => {
    const store = useAgentStore();
    store.list = [
      agent({ id: "a-img", name: "图片位", avatar: "data:image/webp;base64,xxx" }),
      agent({ id: "a-other", name: "兜底位" }),
    ];
    const w = mount(AgentPicker, { props: { agentIds: ["a-other"] } });
    expect(w.findAllComponents({ name: "EntityAvatar" }).length).toBe(1);
    expect(w.text()).not.toContain("图片位");
  });
});
