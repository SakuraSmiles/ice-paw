// crossSession — MA-3 跨会话来件的消费侧解析（纯函数）。
//
// 后端消费回合物化的 user 消息带稳定来源标注头（harness/inbox.rs
// compose_incoming_text，唯一组装点）：
//   [来自会话「{title}」的 agent {name}｜如需回复用 send_message_to_session 工具，target={conv_id}]\n\n{content}
// 本模块是它的镜像解析器：聊天区据此渲染 incoming 卡（来源标注头 + 正文 +
// 点击跳源会话），与后端常量 INCOMING_PREFIX_HEAD 保持同一前缀锚。

/** 来源标注头前缀锚（与后端 INCOMING_PREFIX_HEAD 逐字一致；改动两边同步） */
export const INCOMING_PREFIX_HEAD = "[来自会话「";

/** 解析结果：null = 非来件（普通用户消息，原样渲染） */
export interface IncomingInfo {
  sourceTitle: string;
  agentName: string;
  sourceConvId: string;
  /** 标注头之后的正文（已去头空行；理论上恒非空——后端投递校验拒空内容） */
  body: string;
}

/**
 * 解析来件消息全文。非 `[来自会话「` 开头、或头格式对不上（后端格式演进 /
 * 用户手写相似文本）→ null 诚实降级为普通消息——宁可少渲染一张卡，
 * 不把普通消息错标成跨会话来件。
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
