// crossSession — MA-3 跨会话来件的消费侧解析（纯函数）。
//
// 消费回合物化的 user 消息带**双块结构**（harness/inbox.rs compose_incoming_blocks，
// 唯一组装点）：[Text(来源标注), Text(正文)]；messages 行同时落 incoming_source
// 元数据列（migration 53）+ user_message 事件 payload（derive 透传）。
//
// 解析两级：
// 1. **元数据优先**（incomingInfoOf）：消息带 incoming_source 元数据 → 直接
//    组装 IncomingInfo（权威数据源，格式演进免疫）；
// 2. **文本解析兜底**（parseIncomingText）：对扁平 content（messages.content
//    检索面 / 旧版本消息）按稳定来源标注头格式解析：
//    [来自会话「{title}」的 agent {name}｜如需回复用 send_message_to_session 工具，target={conv_id}]\n\n{content}
//    与后端 compose_incoming_annotation 逐字镜像（格式改动两边同步）。
// 两级都 miss → null 诚实降级为普通消息渲染。

import type { Message } from "../types";

/** 来源标注头前缀锚（与后端 INCOMING_PREFIX_HEAD 逐字一致；改动两边同步） */
export const INCOMING_PREFIX_HEAD = "[来自会话「";

/** 解析结果：null = 非来件（普通用户消息，原样渲染） */
export interface IncomingInfo {
  sourceTitle: string;
  agentName: string;
  sourceConvId: string;
  /** 标注头之后的正文（理论上恒非空——后端投递校验拒空内容） */
  body: string;
}

/**
 * 消息级解析（元数据优先 + 文本兜底）：incoming 卡渲染的统一入口。
 *
 * - 有 incoming_source 元数据 → 权威组装（body 取 content_blocks 的末 Text 块，
 *   与后端双块结构的正文块对位；blocks 解析失败回退 content 全文剥头）；
 * - 无元数据 → parseIncomingText 文本解析（legacy 双块前的扁平消息 / 元数据
 *   坏行降级路径）。
 */
export function incomingInfoOf(msg: Pick<Message, "content" | "content_blocks" | "incoming_source">): IncomingInfo | null {
  const meta = msg.incoming_source;
  if (meta && meta.source_conversation_id && meta.source_conversation_title && meta.source_agent_name) {
    return {
      sourceTitle: meta.source_conversation_title,
      agentName: meta.source_agent_name,
      sourceConvId: meta.source_conversation_id,
      body: incomingBodyFromBlocks(msg.content_blocks) ?? incomingBodyFromFlat(msg.content),
    };
  }
  return parseIncomingText(msg.content);
}

/** 双块结构的正文块：content_blocks 里最后一个 Text 块（标注块在后端恒为首块）。
 *  解析失败 / 无 Text 块 → null（调用方回退扁平剥头）。 */
function incomingBodyFromBlocks(blocksJson: string | null | undefined): string | null {
  if (!blocksJson) return null;
  let blocks: unknown;
  try {
    blocks = JSON.parse(blocksJson);
  } catch {
    return null;
  }
  if (!Array.isArray(blocks)) return null;
  let body: string | null = null;
  for (const b of blocks) {
    if (b && typeof b === "object" && (b as { type?: unknown }).type === "text") {
      const t = (b as { text?: unknown }).text;
      if (typeof t === "string") body = t;
    }
  }
  return body;
}

/** 扁平 content 的正文段：首个 `]\n\n` 之后（标注头收束锚）。 */
function incomingBodyFromFlat(content: string | null | undefined): string {
  if (!content) return "";
  const headEnd = content.indexOf("]\n\n");
  return headEnd < 0 ? content : content.slice(headEnd + 3);
}

/**
 * 解析扁平来件文本（legacy 兜底路径）。非 `[来自会话「` 开头、或头格式对不上
 * （后端格式演进 / 用户手写相似文本）→ null 诚实降级为普通消息——宁可少渲染
 * 一张卡，不把普通消息错标成跨会话来件。
 */
export function parseIncomingText(text: string | null | undefined): IncomingInfo | null {
  if (!text || !text.startsWith(INCOMING_PREFIX_HEAD)) return null;
  // 头结构：[来自会话「title」的 agent name｜…target=convId]，以首个 "]"+\n\n 收束
  const headEnd = text.indexOf("]\n\n");
  if (headEnd < 0) return null;
  const head = text.slice(0, headEnd); // 不含 ] 与 \n\n
  // title：首个 「…」
  const titleEnd = head.indexOf("」");
  if (titleEnd < 0) return null;
  const sourceTitle = head.slice(INCOMING_PREFIX_HEAD.length, titleEnd);
  // agent 名：」的 agent 之后、｜ 之前
  const agentPart = head.slice(titleEnd + "」的 agent ".length);
  const barIdx = agentPart.indexOf("｜");
  if (barIdx < 0) return null;
  const agentName = agentPart.slice(0, barIdx);
  // 源会话 id：头内 target= 之后（取到头尾，头内无 ] 故无歧义）
  const targetIdx = head.indexOf("target=");
  if (targetIdx < 0) return null;
  const sourceConvId = head.slice(targetIdx + "target=".length);
  if (!sourceTitle || !agentName || !sourceConvId) return null;
  return { sourceTitle, agentName, sourceConvId, body: text.slice(headEnd + 3) };
}
