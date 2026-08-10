// glmMcpTemplates.ts — GLM Coding Plan 套餐自带的 MCP 服务模板
//
// 4 个 GLM MCP 服务（官方文档核实）：
// - 3 个 Remote（streamable HTTP）：联网搜索 / 网页读取 / 开源仓库
//   端点 https://open.bigmodel.cn/api/mcp/{name}/mcp，Authorization: Bearer KEY
// - 1 个 Local（stdio）：视觉理解 —— npx @z_ai/mcp-server，env Z_AI_API_KEY + Z_AI_MODE
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
  /** 类型标签文案（远程 / 本地） */
  badge: string;
  /** 是否本地（stdio）—— 决定 API Key 注入位置与前端提示 */
  local: boolean;
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
    local: false,
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
  {
    key: "glm-vision",
    name: "GLM 视觉理解",
    description: "图像理解（本地 npx @z_ai/mcp-server）",
    badge: "本地",
    local: true,
    build: (apiKey) => ({
      name: "GLM 视觉理解",
      description: "图像理解（本地 npx @z_ai/mcp-server）",
      command: "npx",
      args: ["@z_ai/mcp-server"],
      env: { Z_AI_API_KEY: apiKey, Z_AI_MODE: "ZHIPU" },
      transport: "stdio",
      trust_level: "trusted", // 对齐 builtin-playwright 的本地工具信任级
      enabled: true,
      scope: "global",
    }),
  },
];
