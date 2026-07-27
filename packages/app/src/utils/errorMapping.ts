// 前端错误映射工具（REQ-AGENT-031）
//
// 设计原则：
//   - 与后端 `harness::error_mapping::agent_user_message` 镜像维护
//   - 后端已经通过 `kind` 字段把 AppError 归类为稳定字符串（"duplicate" /
//     "agent_not_found" / "no_provider_available" ...），前端拿到 `{kind,
//     message}` 后只需按 kind 查表就能拿到本地化提示
//   - 不匹配时返回 null，调用方走默认兜底文案
//
// 修改这里时，请同步检查后端 `src-tauri/src/harness/error_mapping.rs` 的
// `agent_user_message` 与 `error_kind`（两边要保持一致）。

/** 错误 kind → 中文 toast 文案 */
const AGENT_USER_MESSAGE: Record<string, string> = {
  // REQ-AGENT-019：名称重复
  duplicate: "Agent 名称已存在",
  // REQ-AGENT-031：Agent 资源不存在
  agent_not_found: "Agent 不存在",
  // REQ-AGENT-033：全部 provider 都不可用
  no_provider_available: "无可用 Provider，请在设置中检查",
  // 后端 kind（来自 error_mapping::error_kind）
  provider_not_configured: "Provider 未配置",
  rate_limited: "AI 服务繁忙，请稍后重试",
  stream_connection_failed: "AI 服务连接失败，请检查网络",
  cancelled: "操作已取消",
  // 后端 AppError::Validation
  validation: "参数校验失败，请检查输入",
};

/**
 * REQ-AGENT-031：把 invoke 抛出的错误归一为 Agent 模块的 toast 文案。
 *
 * @param err invoke 抛出的任意值（Error / `{ kind, message }` / string）
 * @returns 若匹配到 Agent 错误则返回中文提示；否则返回 null（调用方兜底）
 */
export function agentUserMessage(err: unknown): string | null {
  let kind: string | undefined;
  if (err instanceof Error) {
    // 兼容 bridge.ts 已用 `[bridge.op/kind] ...` 包装后的 Error
    const m = err.message.match(/^\[bridge\.[^\]]+\/([a-z_]+)\]/);
    if (m && m[1]) kind = m[1];
  } else if (typeof err === "object" && err !== null) {
    const obj = err as Record<string, unknown>;
    if (typeof obj.kind === "string") kind = obj.kind;
  }
  if (kind && Object.prototype.hasOwnProperty.call(AGENT_USER_MESSAGE, kind)) {
    return AGENT_USER_MESSAGE[kind]!;
  }
  return null;
}