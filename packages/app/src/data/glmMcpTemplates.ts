// glmMcpTemplates.ts — GLM Coding Plan 套餐自带的 MCP 服务模板
//
// 3 个 GLM MCP 服务（官方文档核实），全部 Remote（streamable HTTP）：
// 联网搜索 / 网页读取 / 开源仓库
// 端点 https://open.bigmodel.cn/api/mcp/{name}/mcp，Authorization: Bearer KEY
//
// 视觉理解模板已撤（2026-08-27 视觉两档制）：@z_ai/mcp-server 内置 GLM-4.6V
// 不可控（随包升级漂移）、Coding 套餐专属，且与「设置-通用-视觉读取」配置链
// 职责重复——平台兜底已由显式配置链承载，不再需要借 MCP env 的暗通道。
//
// 模板只做「预填」：build(apiKey) 产出可直接 create 的配置，用户仍可在表单二次编辑。
// API Key 明文存 server 配置（headers / env，同安全级别），后续可升级 key_slot / 脱敏。
import type { NewMcpServer } from "../types";

/** 一个 GLM MCP 服务模板 */
export interface GlmMcpTemplate {
  /** 模板唯一 key */
  key: string;
  /** 展示名 */
  name: string;
  /** 一句话说明 */
  description: string;
  /** 类型标签文案（远程） */
  badge: string;
  /** 用 API Key 构造一份可直接创建的配置（不含 id，调用方 randomUUID） */
  build: (apiKey: string) => Omit<NewMcpServer, "id">;
}

const GLM_MCP_BASE = "https://open.bigmodel.cn/api/mcp";

/** 3 个 Remote 服务共用构造：streamable HTTP + Bearer 认证 */
function remoteTemplate(
  key: string,
  name: string,
  description: string,
  serverName: string,
): GlmMcpTemplate {
  return {
    key,
    name,
    description,
    badge: "远程",
    build: (apiKey) => ({
      name,
      description,
      command: "", // 远程传输无启动命令；后端 transport=http 分支不读
      transport: "http",
      url: `${GLM_MCP_BASE}/${serverName}/mcp`,
      headers: { Authorization: `Bearer ${apiKey}` },
      trust_level: "untrusted",
      enabled: true,
      scope: "global",
    }),
  };
}

export const GLM_MCP_TEMPLATES: GlmMcpTemplate[] = [
  remoteTemplate("glm-web-search", "GLM 联网搜索", "实时联网搜索（web_search_prime）", "web_search_prime"),
  remoteTemplate("glm-web-reader", "GLM 网页读取", "抓取并理解网页正文（web_reader）", "web_reader"),
  remoteTemplate("glm-zread", "GLM 开源仓库", "检索开源仓库代码（ZRead）", "zread"),
];
