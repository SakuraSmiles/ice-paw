---
name: mcp-tool-system
description: MCP 工具系统完整迁移与前端渲染开发计划
metadata:
  type: project
---

# MCP 工具系统迁移计划

## 总体架构

```
loop_engine（不变）
    │
    └── McpClient trait（统一接口，替代旧的 Tool trait）
         ├── InternalMcpClient  → 内置 read_file / list_directory（Rust 原生）
         └── ExternalMcpClient  → 用户配置的 MCP Server（stdio JSON-RPC）
```

## 阶段拆分

---

### Phase 1：后端 MCP 基础设施（2 次会话）

**目标**：搭建 McpClient trait，包装现有内置工具，删除旧 Tool trait。

| 任务 | 文件 | 内容 |
|---|---|---|
| 1.1 | **新增** `harness/mcp/types.rs` | MCP 协议 JSON-RPC 消息类型（~80 行） |
| 1.2 | **新增** `harness/mcp/client.rs` | `McpClient` trait + `McpRegistry`（~150 行） |
| 1.3 | **新增** `harness/mcp/internal.rs` | `InternalMcpClient` 包装现有 read_file/list_directory（~100 行） |
| 1.4 | **新增** `harness/mcp/mod.rs` | 模块入口 |
| 1.5 | **删除** `harness/tool_registry/` | 移除旧的 Tool trait + ToolRegistry + authority + scoring + builtin（~400 行） |
| 1.6 | **修改** `harness/loop_engine.rs` | 引用更新为 McpRegistry |
| 1.7 | **修改** `commands/chat_cmd.rs` | 移除旧的 ToolRegistry 注入，替换为 McpRegistry |
| 1.8 | **运行测试** | 确保 324 个测试全部通过 |

**涉及文件**：新增 ~330 行，删除 ~400 行，净减 ~70 行

---

### Phase 2：外部 MCP Server 连接器（2 次会话）

**目标**：ExternalMcpClient 支持 stdio 子进程通信。

| 任务 | 文件 | 内容 |
|---|---|---|
| 2.1 | **新增** `harness/mcp/external.rs` | `ExternalMcpClient` — stdio 子进程管理 + JSON-RPC 收发（~200 行） |
| 2.2 | **修改** `harness/mcp/mod.rs` | 注册 ExternalMcpClient 启动/关闭生命周期 |
| 2.3 | **修改** `lib.rs` | setup 阶段读取 MCP 配置，启动外部 Server |
| 2.4 | **新增** `db/migrations/25_mcp_servers.sql` | 持久化 MCP Server 配置（命令/参数/白名单） |
| 2.5 | **新增** `commands/mcp_cmd.rs` | 管理 MCP Server 的 Tauri commands |
| 2.6 | **测试** | 集成测试：启动一个外部 Server + 调用工具 |

---

### Phase 3：前端工具渲染（2 次会话）

**目标**：用户能看到工具调用过程和结果。*此阶段不依赖 Phase 2 完成，可并行开发。*

| 任务 | 文件 | 内容 |
|---|---|---|
| 3.1 | **修改** `ChatMessages.vue` | 渲染 `tool_use` 块（🔧 工具名 + 参数） |
| 3.2 | **修改** `ChatMessages.vue` | 渲染 `tool_result` 块（结果内容，可折叠） |
| 3.3 | **修改** `ChatMessages.vue` | 渲染 `thinking` 块（Anthropic 思考过程） |
| 3.4 | **修改** `stores/chat.ts` | 处理 `chat:tool-call-start/delta/end` 事件 |
| 3.5 | **修改** `stores/chat.ts` | 处理 `chat:thinking` 事件 |
| 3.6 | **新增** `ToolAuthDialog.vue` | 工具授权确认弹窗（路径确认/拒绝） |

---

### Phase 4：用户配置界面（1 次会话）

**目标**：用户在设置页面管理 MCP Server。

| 任务 | 文件 | 内容 |
|---|---|---|
| 4.1 | **新增** `pages/settings/McpSettings.vue` | MCP Server 列表 + 添加/编辑/删除 |
| 4.2 | **修改** `SettingsLayout.vue` | 导航菜单新增"MCP 工具" |
| 4.3 | **修改** `router/index.ts` | 新增 `/settings/mcp` 路由 |

---

### Phase 5：收尾清理（1 次会话）

| 任务 | 内容 |
|---|---|
| 5.1 | 删除 `tool_registry/` 目录（确认无引用残留） |
| 5.2 | 删除 `infra/protocol.rs` 中的 `ToolDef` 类型（移动到 `mcp/types.rs`） |
| 5.3 | 全量测试 + CI 验证 |
| 5.4 | 文档更新 |

---

## 优先级建议

| 优先级 | 阶段 | 原因 |
|---|---|---|
| P0 | **Phase 1** | 基础设施，不然后面都做不了 |
| P1 | **Phase 3** | 可以和 Phase 1 并行，不依赖外部 MCP |
| P2 | **Phase 2 + 4** | 外部 MCP + 配置 UI，按需推进 |
| P3 | **Phase 5** | 清理善后 |

## 关键决策记录

- 内部工具 `read_file` / `list_directory` 通过 `InternalMcpClient` 保留 Rust 原生实现
- 外部工具通过 `ExternalMcpClient` + stdio JSON-RPC 接入
- 对外暴露统一 `McpClient` trait，loop_engine 不区分内外
- 授权/超时/错误处理在 trait 层统一
- Phase 3 前端渲染可独立于 Phase 2 进行
