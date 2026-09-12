# 工具集权限（tool_scopes）设计真相源

2026-09-12 拍板批次。生产实案驱动：用户给视觉 agent（vision-eye）接 UE5 MCP 后，
发现 agent 侧没有任何机制把外部 server 的工具面开放/收窄给特定 agent——只有
`enabled_tools` 白名单（t 前缀名、会漂移、外部工具枚举困难）和 server 全局开关两档，
缺「按 agent 的工具集范围」这一层。

## 三层权限模型（勿混淆）

| 层 | 机制 | 本批 |
|---|---|---|
| 组装可见性 | `tool_scopes`（本设计） | ✅ |
| 运行时授权 | AuthScope 四档弹卡（once/this_dir/this_tool/this_server） | 已有 |
| server 全局开关 | `mcp_servers.enabled` | 已有 |

**收窄 ≠ 安全边界**：看得见 ≠ 免问——审批卡才是安全边界。收窄的价值 = 降噪 /
省 token / 减误用 / 缩小默认攻击面。文案与心智按此口径（AgentForm 提示行）。

## 模型（用户拍板四项）

- **粒度**：单字段 `tool_scopes: Option<Vec<String>>`（agent.yaml 块 + DB 列镜像，
  migration 56），条目三形态：
  - `group:<组键>` —— 内置工具组（七组固定名单快照）
  - `server:<server 配置 id>` —— 外部 MCP server 全部工具（按归属探针
    `server_config_id()` 匹配，t 前缀漂移免疫）
  - 裸工具名 —— 逃生舱（组没覆盖的单点）
- **缺省 / 空 = 全部**（默认全开，与 enabled_tools「空 ≡ 全开」同约定）；与
  `enabled_tools` **串联**：先 scopes 过滤、再名单过滤（交集语义）。存量 agent
  零迁移零行为变化。
- **组语义 = 固定名单快照**：组 = 建组时固化名单，**新增工具不自动进组**——安全
  面不静默扩大；要放行新工具须显式改 scopes（用户可感知）。测试
  `groups_cover_all_registered_tools` 锁定全覆盖 + 无重复。
- **「other」不是 scope 组**：设置页展示兜底而已，`group:other` 视为死条目。

## 单一真相源

- 后端：`harness/mcp/tool_scopes.rs`（`TOOL_GROUPS` 七组名单 + `scope_allows` /
  `parse_scope_entry` / `dead_scope_entries` 纯函数 + `tool_group_of` 反查）。
  组键：`files` 文件与命令 / `web` 网页 / `kb` 知识库 / `attach` 附件与引用 /
  `docx` Word 文档 / `config` 配置与计划 / `screen` 屏幕读写。
- 前端：`data/toolGroups.ts` 只放**展示名**（组键→中文标签）；组成员计数从
  `list_builtin_tools` 响应的 `group` 字段聚合，不在前端手抄名单。
- McpSettings 分组数据源已切后端（group 字段），BUILTIN_TOOL_META 只剩中文
  描述文案层。

## 写入通道（旋钮唯一写入通道不变式）

- **唯一通道**：`set_agent_tool_scopes` 命令（agent_yaml.rs）——yaml 块补丁
  （整文件 serde 回读校验）+ 原子写 + **DB 列镜像双写**（镜像同步不变式同
  enabled_tools：组装期收窄读 DB 行，只写 yaml 不清 DB = 旧值复活）。
- `Some(items)` → 写 `tool_scopes: [...]`（yaml_flow_str 对 `group:files` 类
  冒号条目自动加引号防 map 解析）；`None`/空 → 摘除块 + DB 置 NULL。
- **不进 AgentUpdate**（update_agent 只管出生证）；agent_cmd update 路径显式
  `tool_scopes: None` 带注释。
- 前端 AgentForm「工具」区块（仅编辑态渲染）：默认全开胶囊 ↔ 自定义多选
  （七组 + 每台外部 server 一项），随表单主「保存」显式提交（草稿未动不写）；
  已保存死条目（已删 server / 裸名）以 chip 保留防丢、可单独摘。

## 唯一权威与旧白名单并入（2026-09-12 二批，用户拍板「并列即歧义」）

初版把 enabled_tools 与 tool_scopes 的交集语义用提示行并列展示（「与工具集
范围取交集生效」），用户实测反馈：两套机制并排 = 「到底配置文件生效还是 UI
生效」的歧义，提示行是创可贴。根治 = **AgentForm「工具」区块是工具面的唯一
编辑权威**：

- **打开即并入**：编辑态草稿种子 = `tool_scopes ∪ enabled_tools(非平台元工具)`
  ——白名单条目以 chip 呈现（同死条目形态，可单独摘）；平台元工具过滤
  （恒可见无需表达；vision-eye 类「3 平台工具白名单」打开后零噪音）。
- **保存即接管**：工具面草稿变更（scopesDirty，含切「全部工具」）→
  `setToolScopes(final)` 后 `setEnabledTools(null)`（**次序锁：先写后摘**——
  中间态两侧并设 = 交集，只会更窄不会放宽）。提示行从「交集解释」换为接管
  声明：「保存工具面改动后将移除白名单，以本区块为准」。
- **零强制迁移**：不动工具面（含只改名字）保存 → 两个旋钮零写入，白名单
  原样生效（存量 agent 不被无关保存顺手改写）。
- **提案通道同规则**：update 提案带 tool_scopes 且未显式管理 enabled_tools →
  批准时同步 `setEnabledTools(null)`（交集语义下收窄对白名单 agent 无可见
  效果，同属歧义）；提案显式带 enabled_tools 则各走各的。
- 后端交集语义**保留**：手写 yaml 两边都设的专家场景防御组合（交集 = 更窄
  侧胜出），UI 只是保证用户不落在双设状态。
- ⚠️ 前端 `PLATFORM_TOOL_NAMES`（AgentForm）与 `session_runner::PLATFORM_TOOLS`
  两处同步（平台元工具新增时一起改）。

## 组装期过滤（session_runner）

- `filter_tools_by_scopes`：平台元工具恒保留（PLATFORM_TOOLS：
  propose_config_change / read_agent_config / delegate_to_agent /
  send_message_to_session / list_conversations——跨会话通讯与配置通道不收窄）；
  其余按 `scope_allows` 判定；**server 拥有但被收窄掉的工具注册占位 stub**
  （ServerOwnedStub——防止「工具不在场」与「被收窄」不可区分）。
- 与 enabled_tools 串联（先 scopes 后名单 = 交集）。
- **死条目 warn**：scopes 条目对工具快照零命中（裸名拼错如 `read_kb`——生产
  实案、server 已删、组键拼错）→ `dead_scope_entries` 检出 + warn 日志
  「tool_scopes 死条目」——**权限假象比缺工具危险**，零命中必须可见。
- 收窄生效时 info 披露「tool_scopes 收窄：保留 N」——治看不见，勿删。

## 提案通道（propose_config_change）

- `ProposalAction::CreateAgent / UpdateAgent` 均带 `tool_scopes`（serde
  skip_serializing_if None）。
- guard：工具面变更（enabled_tools 或 tool_scopes 任一 Some/非空，含
  `Some([])` 摘除——摘除也是扩大可见面）→ **Medium** + warning。
- 前端 ConfigProposalCard 批准路径按字段分派：update → 旋钮通道
  `setToolScopes`（空数组 = 摘除原样透传）；create → 先 create（默认全开）
  后 `setToolScopes` 收窄（invocationCallOrder 锁次序）。
- 工具 description 披露：组键七项 + server id 语义 + 固定名单快照（新增工具
  不自动进组）。

## tool_index 高水位（同批独立修复）

`mcp_servers.tool_index` 原用 `MAX(tool_index)+1` 分配——删最高位 server 后
新 server 复用被删编号，存量 enabled_tools 白名单的 `t{idx}_xxx` 静默错绑另一
server 的工具（潜在提权口）。修法：preferences 键 `mcp_tool_index_hwm` 高水位
计数器（种子 = max(计数器, 表内 MAX)，只进不退，删 server 不回收；计数器写失败
不阻塞创建——下次种子逻辑仍以表内 MAX 兜底）。sqlite_sequence 不适用（TEXT
主键无 AUTOINCREMENT）。回归锁 `tool_index_never_reused_after_delete`。

## 维护纪律

1. 新增内置工具：进 `TOOL_GROUPS` 对应组（显式归属——快照语义守门测试会红；
   刻意不进组 = 该工具不沾组 scope 光，放行须裸名）。
2. `list_builtin_tools` 响应 shape 变更：McpSettings / AgentForm 两消费方同步。
3. 组标签只在 `data/toolGroups.ts` 改（McpSettings / AgentForm 共用）。
4. 死条目口径：禁用的 server 工具不进快照 → server 条目算死条目 warn（预期
   行为，文案给「已删除或已禁用」全口径）。
