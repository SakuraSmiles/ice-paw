// MA-3 来件消费侧解析纯函数单测：incomingInfoOf（元数据优先）+ parseIncomingText
// （文本兜底）。关键契约：文本格式与后端 harness/inbox.rs compose_incoming_annotation
// 逐字镜像——格式改动两边同步（此处用后端产出的完整样例锁形状）；元数据路径
// 与后端 IncomingSourceMeta 字段镜像。
import { describe, expect, it } from "vitest";
import { incomingInfoOf, parseIncomingText, INCOMING_PREFIX_HEAD } from "../crossSession";
import type { Message } from "../../types";

/** 后端 compose_incoming_annotation 产出的完整样例（唯一组装点的形状锁） */
const BACKEND_SAMPLE =
  "[来自会话「UE5 材质主控」的 agent 甲｜如需回复用 send_message_to_session 工具，target=conv-abc123]\n\n材质定稿了吗？缺一份金属度参考图。";

/** 后端双块结构的 content_blocks 样例（标注块 + 正文块） */
const BACKEND_BLOCKS = JSON.stringify([
  { type: "text", text: "[来自会话「UE5 材质主控」的 agent 甲｜如需回复用 send_message_to_session 工具，target=conv-abc123]" },
  { type: "text", text: "材质定稿了吗？缺一份金属度参考图。" },
]);

/** 消息替身（Pick<Message, ...> 的最小构造） */
function msgOf(over: Partial<Pick<Message, "content" | "content_blocks" | "incoming_source">> = {}) {
  return {
    content: BACKEND_SAMPLE,
    content_blocks: BACKEND_BLOCKS,
    incoming_source: null,
    ...over,
  };
}

describe("incomingInfoOf（元数据优先）", () => {
  it("有元数据 → 权威组装，不依赖文本格式（正文取 content_blocks 末 Text 块）", () => {
    const info = incomingInfoOf(msgOf({
      // content 故意放一段格式对不上的文本：元数据在场时文本不被信任
      content: "任意扁平文本",
      incoming_source: {
        source_conversation_id: "conv-xyz",
        source_conversation_title: "UE5 材质主控",
        source_agent_name: "甲",
      },
    }));
    expect(info).toEqual({
      sourceTitle: "UE5 材质主控",
      agentName: "甲",
      sourceConvId: "conv-xyz",
      body: "材质定稿了吗？缺一份金属度参考图。",
    });
  });

  it("元数据 + content_blocks 坏 JSON → 正文回退扁平 content 剥头", () => {
    const info = incomingInfoOf(msgOf({ content_blocks: "{broken" }));
    expect(info?.body).toBe("材质定稿了吗？缺一份金属度参考图。");
  });

  it("无元数据 → 回退文本解析（legacy 路径不回退）", () => {
    const info = incomingInfoOf(msgOf({ incoming_source: null }));
    expect(info?.sourceConvId).toBe("conv-abc123");
    expect(info?.body).toBe("材质定稿了吗？缺一份金属度参考图。");
  });

  it("元数据字段残缺（空 id）→ 不按元数据渲染，回退文本解析", () => {
    const info = incomingInfoOf(msgOf({
      incoming_source: {
        source_conversation_id: "",
        source_conversation_title: "UE5 材质主控",
        source_agent_name: "甲",
      },
    }));
    expect(info?.sourceConvId, "回退文本解析而非拿空 id 组装").toBe("conv-abc123");
  });
});

describe("parseIncomingText（文本兜底）", () => {
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
