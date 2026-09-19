// ChatInput.steer.test.ts — Steer 插话衔接态（§11 过渡提示的轻量版）回归锁：
// 回合 A 被 abort 收尾（chat:done → sending=false）到回合 B 起跑之间存在 3s+
// 静默窗（后端 CHAIN_HEAD_QUIET 聚合连发）——此窗输入区不该长得跟真空闲一样：
// hint 显示「插话排队中，即将继续…」+ mini spinner + wrapper 保留 is-sending 边框。
// 判据守卫：排队角标必须命中当前会话消息（messages 即当前会话列表，切走不显）；
// sending=true 让位生成中形态（回合 B 起跑瞬间角标可能尚未被 prune 摘除）。
import { describe, it, expect, beforeEach } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { mount } from "@vue/test-utils";
import { nextTick } from "vue";
import ChatInput from "../ChatInput.vue";
import { useChatStore } from "../../../stores/chat";
import { useProjectStore } from "../../../stores/project";

function msg(id: string, role: "user" | "assistant", content: string) {
  return {
    id, conversation_id: "c1", role, content, content_blocks: "[]",
    token_count: null, error: null, created_at: "2026-09-19 00:00:00",
    rowid: 1, model: null,
  } as never;
}

describe("ChatInput Steer 衔接态", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    // 挡 projectStore 惰性 load（invoke 全局 mock resolve undefined，同 refs 测试）
    const projects = useProjectStore();
    projects.list = [];
    projects.loaded = true;
  });

  it("静默窗内（!sending ∧ 角标命中当前会话消息）→ 衔接文案 + spinner + is-sending 边框", () => {
    const chat = useChatStore();
    chat.messages = [msg("m1", "user", "插话内容"), msg("m2", "assistant", "被中止的回复")];
    chat.queuedSteerIds = new Set(["m1"]);
    const w = mount(ChatInput);
    expect(w.find(".input-hint").text()).toContain("插话排队中，即将继续…");
    expect(w.find(".input-hint .spin").exists()).toBe(true); // mini spinner（lucide inline-block）
    expect(w.find(".input-wrapper").classes()).toContain("is-sending"); // 边框指示不回落
  });

  it("会话归属守卫：角标命中的消息不在当前会话（用户已切走）→ 不显衔接态", () => {
    const chat = useChatStore();
    chat.messages = [msg("m9", "assistant", "另一会话的消息")];
    chat.queuedSteerIds = new Set(["m1"]); // m1 属于切走的那条会话
    const w = mount(ChatInput);
    expect(w.find(".input-hint").text()).toContain("Enter 发送"); // 常规空闲文案
    expect(w.find(".input-wrapper").classes()).not.toContain("is-sending");
  });

  it("回合 B 起跑（sending=true）→ 让位生成中形态，衔接文案不劫持（角标未摘也不显）", () => {
    const chat = useChatStore();
    chat.messages = [msg("m1", "user", "插话内容")];
    chat.queuedSteerIds = new Set(["m1"]); // prune 由消息事件驱动，起跑瞬间可能仍在
    chat.sending = true;
    const w = mount(ChatInput);
    expect(w.find(".input-hint").text()).not.toContain("插话排队中");
    expect(w.find(".input-wrapper").classes()).toContain("is-sending"); // 生成中本体
    expect(w.find(".btn-stop").exists()).toBe(true);
  });

  it("积压消费完（角标清空）→ 回落常规空闲文案（衔接态随队列清空退出）", async () => {
    const chat = useChatStore();
    chat.messages = [msg("m1", "user", "插话内容")];
    chat.queuedSteerIds = new Set(["m1"]);
    const w = mount(ChatInput);
    expect(w.find(".input-hint").text()).toContain("插话排队中");
    chat.queuedSteerIds = new Set(); // prune 清空（回合消费完毕）
    await nextTick();
    expect(w.find(".input-hint").text()).toContain("Enter 发送");
    expect(w.find(".input-wrapper").classes()).not.toContain("is-sending");
  });
});
