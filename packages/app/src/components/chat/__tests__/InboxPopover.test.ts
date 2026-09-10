// InboxPopover 组件测试（MA-3 收件箱浮层）。
// 锁定交互契约：
// - 权威数据：挂载/处置后走 list_inbox（badge 本地计数只是气味）
// - 批准 = 即时动作调 respond(true)；失败（会话忙）→ 后端三段式文案进横幅，来件留队
// - 拒绝 = 不可恢复处置 → 两步确认（一次武装 danger 确认键、二次执行、blur 解除）
// - 政策三态 segmented：乐观切 + 失败回滚
import { describe, it, expect, beforeEach, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { bridge } from "../../../api/bridge";
import InboxPopover from "../InboxPopover.vue";
import type { InboxItem, InboxView } from "../../../types";

vi.mock("../../../api/bridge", () => ({
  bridge: {
    inbox: {
      list: vi.fn(),
      respond: vi.fn().mockResolvedValue(undefined),
      setPolicy: vi.fn().mockResolvedValue(undefined),
    },
  },
}));

const mockList = vi.mocked(bridge.inbox.list);
const mockRespond = vi.mocked(bridge.inbox.respond);
const mockSetPolicy = vi.mocked(bridge.inbox.setPolicy);

// EntityAvatar 三级降级链依赖 canvas/storage——组件测试里 stub 掉（行为在专测覆盖）
vi.mock("../../common/EntityAvatar.vue", () => ({
  default: { name: "EntityAvatar", props: ["name", "image", "size"], template: '<span class="avatar-stub" />' },
}));

function item(over: Partial<InboxItem> = {}): InboxItem {
  return {
    message_id: "m1",
    source_conversation_id: "c-src",
    source_conversation_title: "主控",
    source_agent_id: "a1",
    source_agent_name: "甲",
    content: "材质定稿了吗",
    expect_reply: false,
    delivered_at_unix: Math.floor(Date.now() / 1000) - 60,
    ...over,
  };
}

function view(policy: string, items: InboxItem[]): InboxView {
  return { policy, items };
}

async function mountPopo() {
  const w = mount(InboxPopover, { props: { convId: "c1" } });
  await new Promise((r) => setTimeout(r, 0)); // watch immediate → load 完成
  return w;
}

describe("InboxPopover", () => {
  beforeEach(() => {
    mockList.mockReset().mockResolvedValue(view("hold", []));
    mockRespond.mockClear().mockResolvedValue(undefined);
    mockSetPolicy.mockClear().mockResolvedValue(undefined);
  });

  it("渲染权威列表：条目源信息 + 计数 + 期待回复/回复标记", async () => {
    mockList.mockResolvedValue(view("hold", [
      item(),
      item({ message_id: "m2", expect_reply: true, source_agent_name: "乙" }),
      item({ message_id: "m3", is_reply: true, source_agent_name: "丙" }),
    ]));
    const w = await mountPopo();

    expect(mockList).toHaveBeenCalledWith("c1");
    expect(w.findAll(".inbox-item")).toHaveLength(3);
    expect(w.text()).toContain("3 条待处理");
    expect(w.text()).toContain("甲 · 来自「主控」");
    // expect_reply / is_reply 条目各带标记（is_reply pill 优先于 expect_reply），普通条目不带
    const flagTexts = w.findAll(".inbox-reply-flag").map((f) => f.text());
    expect(flagTexts).toHaveLength(2);
    expect(flagTexts).toContain("期待回复");
    expect(flagTexts).toContain("回复");
  });

  it("空队列 → 空态文案", async () => {
    const w = await mountPopo();
    expect(w.text()).toContain("暂无待处理来件");
    expect(w.findAll(".inbox-item")).toHaveLength(0);
  });

  it("批准：respond(true) 即时执行；处置后权威重载", async () => {
    mockList.mockResolvedValue(view("hold", [item()]));
    const w = await mountPopo();

    await w.get(".inbox-act-approve").trigger("click");
    await new Promise((r) => setTimeout(r, 0));

    expect(mockRespond).toHaveBeenCalledWith("c1", "m1", true);
    // 挂载 1 + 处置后重载 1 + refreshInboxCount 计数回正 1（badge 气味回真相）
    expect(mockList).toHaveBeenCalledTimes(3);
  });

  it("批准失败（会话忙）→ 错误横幅显示后端文案，来件留队", async () => {
    mockList.mockResolvedValue(view("hold", [item()]));
    const w = await mountPopo();

    mockRespond.mockRejectedValue(new Error("会话正在生成中：等当前回合结束后再批准"));
    await w.get(".inbox-act-approve").trigger("click");
    await new Promise((r) => setTimeout(r, 0));

    expect(w.text()).toContain("会话正在生成中");
    expect(w.find(".inbox-item").exists()).toBe(true); // 列表未清空
  });

  it("拒绝两步确认：一次武装（文案变确认键）、二次执行、blur 解除", async () => {
    mockList.mockResolvedValue(view("hold", [item()]));
    const w = await mountPopo();
    const btn = w.findAll(".inbox-act").find((b) => b.text() === "拒绝")!;

    // 第一次点击：武装，不执行
    await btn.trigger("click");
    expect(mockRespond).not.toHaveBeenCalled();
    expect(btn.text()).toBe("确认拒绝？");
    expect(btn.classes()).toContain("inbox-act-armed");

    // blur 解除武装（外点解除的键盘等价路径）
    await btn.trigger("blur");
    expect(w.findAll(".inbox-act").find((b) => b.text() === "拒绝")!.text()).toBe("拒绝");

    // 重新武装 → 第二次点击执行
    await btn.trigger("click");
    await btn.trigger("click");
    await new Promise((r) => setTimeout(r, 0));
    expect(mockRespond).toHaveBeenCalledWith("c1", "m1", false);
  });

  it("政策切换：乐观切 + setPolicy；失败回滚 + 错误横幅", async () => {
    mockList.mockResolvedValue(view("hold", []));
    const w = await mountPopo();

    const btnOf = (label: string) => w.findAll(".inbox-policy-btn").find((b) => b.text() === label)!;
    expect(btnOf("需批准").classes()).toContain("active");

    // 成功路径：切自动接收（默认档，label 带「（默认）」标注）
    await btnOf("自动接收（默认）").trigger("click");
    await new Promise((r) => setTimeout(r, 0));
    expect(mockSetPolicy).toHaveBeenCalledWith("c1", "accept");
    expect(btnOf("自动接收（默认）").classes()).toContain("active");

    // 失败路径：切拒收被后端拒 → 回滚 accept + 横幅
    mockSetPolicy.mockRejectedValue(new Error("无法设置收件政策：会话不存在"));
    await btnOf("拒收").trigger("click");
    await new Promise((r) => setTimeout(r, 0));
    expect(btnOf("拒收").classes()).not.toContain("active");
    expect(btnOf("自动接收（默认）").classes()).toContain("active");
    expect(w.text()).toContain("无法设置收件政策");
  });
});
