// Router 化批 P1-5：DelegationCard 的 worker 变体——spawn_worker 复用委派卡，
// 标题语义切换（「临时工作者 X」vs「委派给 X」），状态/跳转体系零改动。
import { describe, it, expect, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import DelegationCard from "../DelegationCard.vue";

vi.mock("../../../stores/agent", () => ({
  useAgentStore: () => ({ getById: () => null }),
}));
vi.mock("./StatusGlyph.vue", () => ({
  default: { name: "StatusGlyph", props: ["status", "size", "label"], template: "<span class='glyph-stub' />" },
}));
vi.mock("../../common/EntityAvatar.vue", () => ({
  default: { name: "EntityAvatar", props: ["name", "image", "size"], template: "<span class='avatar-stub' />" },
}));

const base = {
  agentName: "临时·统计文档·a9a1",
  task: "数 md 文件",
  status: "done" as const,
  childConvId: "c-1",
  finishReason: "end_turn",
  rounds: 3,
};

describe("DelegationCard worker 变体（批 P1-5）", () => {
  it("cardKind=worker 标题显示「临时工作者」，默认仍是「委派给」", () => {
    setActivePinia(createPinia());
    const w = mount(DelegationCard, { props: { ...base, cardKind: "worker" } });
    expect(w.find(".dlg-title").text()).toContain("临时工作者");
    expect(w.find(".dlg-title").text()).toContain("临时·统计文档·a9a1");
    expect(w.find(".dlg-title").text()).not.toContain("委派给");

    const d = mount(DelegationCard, { props: { ...base } });
    expect(d.find(".dlg-title").text()).toContain("委派给");
  });
});
