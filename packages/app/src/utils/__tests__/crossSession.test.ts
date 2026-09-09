// parseIncomingText 纯函数单测（MA-3 来件消费侧解析）。
// 关键契约：与后端 harness/inbox.rs compose_incoming_text 的组装格式逐字镜像——
// 格式改动两边同步（此处用后端产出的完整样例锁形状）。
import { describe, expect, it } from "vitest";
import { parseIncomingText, INCOMING_PREFIX_HEAD } from "../crossSession";

/** 后端 compose_incoming_text 产出的完整样例（唯一组装点的形状锁） */
const BACKEND_SAMPLE =
  "[来自会话「UE5 材质主控」的 agent 甲｜如需回复用 send_message_to_session 工具，target=conv-abc123]\n\n材质定稿了吗？缺一份金属度参考图。";

describe("parseIncomingText", () => {
  it("后端样例逐字段解析（形状锁：title/agent/源会话 id/正文）", () => {
    const info = parseIncomingText(BACKEND_SAMPLE);
    expect(info).toEqual({
      sourceTitle: "UE5 材质主控",
      agentName: "甲",
      sourceConvId: "conv-abc123",
      body: "材质定稿了吗？缺一份金属度参考图。",
    });
  });

  it("正文多行完整保留（含 Markdown / 空行）", () => {
    const info = parseIncomingText(
      "[来自会话「t」的 agent n｜如需回复用 send_message_to_session 工具，target=c1]\n\n第一行\n\n- 列表项\n```rust\nfn x() {}\n```",
    );
    expect(info?.body).toBe("第一行\n\n- 列表项\n```rust\nfn x() {}\n```");
  });

  it("非来件（普通用户消息）→ null 诚实降级", () => {
    expect(parseIncomingText("普通消息")).toBeNull();
    expect(parseIncomingText("[附件 spec.pdf] 已上传")).toBeNull();
    expect(parseIncomingText("")).toBeNull();
    expect(parseIncomingText(null)).toBeNull();
    expect(parseIncomingText(undefined)).toBeNull();
    // 前缀出现但不在开头（正文里提到来件格式）不算来件
    expect(parseIncomingText("正文里提到 " + INCOMING_PREFIX_HEAD + "x」…")).toBeNull();
  });

  it("格式残缺（头没收束/缺段）→ null 而非猜测", () => {
    // 无 ]\n\n 收束（流式截断/手写一半）
    expect(parseIncomingText("[来自会话「t」的 agent n｜target=c1] 但没有换行结构")).toBeNull();
    // 缺 」
    expect(parseIncomingText("[来自会话「t 的 agent n｜target=c1]\n\nx")).toBeNull();
    // 缺 ｜ 分隔
    expect(parseIncomingText("[来自会话「t」的 agent n，target=c1]\n\nx")).toBeNull();
    // 缺 target=
    expect(parseIncomingText("[来自会话「t」的 agent n｜如需回复]\n\nx")).toBeNull();
  });

  it("字段为空 → null（后端不产空字段，防手写伪造）", () => {
    expect(parseIncomingText("[来自会话「」的 agent n｜，target=c1]\n\nx")).toBeNull(); // 空 title
    expect(parseIncomingText("[来自会话「t」的 agent ｜，target=c1]\n\nx")).toBeNull(); // 空 agent 名
    expect(parseIncomingText("[来自会话「t」的 agent n｜，target=]\n\nx")).toBeNull(); // 空 target
  });
});
