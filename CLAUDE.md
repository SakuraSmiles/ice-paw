# CLAUDE.md — ice-paw 项目指引

## 项目概述
IcePaw — 本地优先的 LLM 对话工作站。Tauri v2 (Rust) + Vue 3 (TypeScript) 桌面应用。
当前版本：`0.5.2`。

## 设计规则（用户拍板，勿翻案）

**配置放置阶梯**——想新增任何配置项，先按序过四层，落不进 L1-L3 才考虑 L4：
- **L1 好默认**：能靠默认值 + 自适应解决的（如预算 3× 窗口、摘要自适应额度），根本不成为配置
- **L2 状态上屏**：用户需要「看见」的做成 HUD/胶囊/toast（如预算 pill），不是可编辑字段
- **L3 agent.yaml**：专家旋钮进 yaml +「打开 agent.yaml」入口 + 配置提案通道（propose_config_change / set_agent_yaml_field）
- **L4 表单**：只放出生证（name / id / model / key / workspace）

一句话：**状态上屏，配置进 yaml，表单只管出生证**。透明度问题（看不见）≠ 可配置性问题（不能改），勿用加表单字段治看不见——此为用户多次纠正的系统性偏差（最近一次 2026-08-16，P7 回滚 AgentForm 高级区）。

**视觉规范三则**（2026-08-20 表现层走查后拍板）：

1. **色彩三层架构**——交互层藏青单色（浅 #1E4976 / 深 #4E80C0，双锚点十一档）+ 语义层三色（danger/success/warning，已建勿动）+ 身份层点缀（--ip-accent-agent 紫等）。info 并入主色（信息提示=品牌蓝，不新增第五色）。品牌色单一真相源在 tokens.css，global.css 不得再定义 --ip-primary-*。tokens 三主题区（:root 浅 / data-theme 暗 / prefers-scheme 自动暗）改动必须三区同步——2026-08-20 实测：单区插入静默失败致暗色聊天区整体回落浅色值。
2. **图标规则（系统级零 emoji）**——一律 Lucide（lucide-vue-next），24×24 stroke 线性语言、currentColor 单色；禁止 emoji 出现在 UI 任何位置（含按钮/标签/占位文案）。语义档位（原 🟢🟡🔴）用「语义色圆点 + 文字」或 Lucide shield 系图标。例外区仅两个：ProviderIcon 品牌 glyph（simple-icons 填充式，集中管理）+ EntityAvatar 哈希色板（数据色板）。存量 105 个手写 SVG 渐进替换，新代码一律 Lucide。
3. **字号与间距**——字号走 --ip-text-* 九档梯度（display40/h1-28/h2-22/h3-18/body-lg-17/body-15/body-sm-13/caption-12/**micro-11**/code-14）；micro 是最小合法档（~86 处幽灵字号已于 UI-4 批次 A 收编）；**9px/10px 是可读性黑洞，禁止出现**，新代码小字一律 micro。间距：布局级（页面 padding/卡片内边距/区块 gap/列表行距）走 --ip-spacing-* 令牌，光学微调位（图标间隙 2-4px）允许字面量；新代码布局间距强制令牌。字体家族一律 --ip-font-sans/mono/display，勿直写字体名。

4. **字体本地化（产品哲学）**——IcePaw 是本地优先产品，**禁止任何网络字体加载**（Google Fonts/CDN 字体一律不用）；字体经 @fontsource npm 包或 assets/fonts 自托管 woff2 打进安装包。中文字体注意子集化（只带实际用到的字重）。首启离线环境的观感是验收线。

5. **z-index 阶梯令牌化**——层叠层级一律 var(--ip-z-*)（base/dropdown/popover/modal/toast 等 tokens 已定义），禁止裸数字 z-index。浮层嵌套时用「相对层级」思路（父层提高，子层跟父），不做 9999 军备竞赛。

6. **加载态三档规范**——按场景选型：**首屏/首载 → 骨架屏**（shimmer，侧栏样式为模板）；**列表刷新 → 顶部细进度条**（不顶开内容）；**操作等待 → 按钮内 spinner/文案变化**（如"保存中…"）。禁止全屏遮罩 loading（除模态流程必须阻塞时）。

7. **图片内容规范**——气泡附件图：max-width 限定 + 圆角 --ip-radius-lg + loading=lazy + 点击进预览器；头像：EntityAvatar 三级降级链（勿绕过）；图片预览器须支持键盘 ←→ 翻张/Esc 关闭；加载失败用中性占位块 + "图片不可用"文案，不留破图图标。

8. **文案规范（ux writing）**——直接、克制、不感叹、不用语气词（哦/啦/呢）；对用户称"你"不用"您"；错误文案三段式：**发生了什么 + 为什么 + 怎么办**（error_mapping 已是此形状，前端展示对齐）；域术语固定写法：会话/Agent（大写 A）/委派/轨迹/预算/知识库（KB 首次出现标注）；数字与单位之间空格（"3.2 MB"、"214 ms"）；时间一律相对时（timeAgo）+ hover 绝对时。

9. **无障碍基线**——键盘可达所有操作（Tab/Enter/Esc/方向键，弹层焦点管理）；:focus-visible 全局焦点环（组件不得用 outline:none 抵消而不补替代）；文本对比度 AA（4.5:1，大字 3:1）；reduced-motion 全局降级已有（tokens §12），新增动效必须跟随。

**工程原则（2026-08-21 头像裁剪器教训）**：成熟库优先——拖拽/缩放/虚拟滚动等指针交互密集的领域，边缘情况（钳制边界/pointer capture/时序）远比表面复杂，自研数学易连翻车（当日前科：ready 死锁→长图锁死→再拖不动，三轮真机才换 vue-cropper 终结）。判定：npm 有维护活跃、生产验证充分（如若依同款）的库 → 直接用，自研只做业务薄壳。

## 构建命令

### 先看你在哪个平台（勿跨平台照搬命令）
- **Windows**：sodium 用仓库内预编译（下述 SODIUM_LIB_DIR）；端口占用 `taskkill //F //PID <pid>`。
- **macOS**：sodium 静态链接 brew 的 `libsodium.a`（**推荐**：仓库根 `.cargo/config.toml` 设 `SODIUM_LIB_DIR=/opt/homebrew/lib`，机器级配置已 gitignore；勿用 `SODIUM_USE_PKG_CONFIG`——pkg-config 分支只会动态链接，产物依赖目标机装了 brew）。⚠️ `SODIUM_LIB_DIR` 与 `SODIUM_USE_PKG_CONFIG` 互斥，crates 的 build.rs 遇双设直接 panic。crates.io 直连不稳时同文件配 rsproxy 镜像。端口占用 `lsof -ti:1420 | xargs kill`。prepare 脚本按平台自动分派（`node scripts/prepare.mjs`：win→ps1，mac/linux→sh）。
- **跨机器传代码只用 git clone，勿用压缩包**——机器级 `src-tauri/.cargo/config.toml` 被 gitignore 挡住但 tarball 会带出来（2026-08-20 实测：Windows 的 D:/ 路径毒死 mac 构建）。

### Rust（Windows）
```bash
# 一劳永逸（推荐）：复制 packages/app/src-tauri/.cargo/config.toml.example → config.toml
# 填本机 sodium-prebuilt 路径（gitignored 机器级文件）。tauri:dev/build 的 cargo 在
# src-tauri 下运行会自动吃到，无需手传 env。⚠️ 机器间不同步（git 不带 gitignored 文件），
# 换机/重拉工作区后要重建；仓库根跑 `cargo --manifest-path` 时按 CWD 找配置吃不到它，仍需显式传。

# cargo check（推荐）——需显式传 sodium 库路径
SODIUM_LIB_DIR="D:/workspace/ice-paw/sodium-prebuilt/libsodium/x64/Release/v143/static" \
SODIUM_STATIC=true \
cargo check --manifest-path packages/app/src-tauri/Cargo.toml

# cargo check --tests 验证测试编译
# cargo test --lib 本地可跑（comctl32 v6 manifest 已由 build.rs 注入 test harness）
#   ⚠️ 曾长期误记为「sodium DLL STATUS_ENTRYPOINT_NOT_FOUND」，真根因是 lib #[test]
#   harness 缺 Common-Controls v6 manifest（TaskDialogIndirect 静态导入），与 sodium 无关。
#   现状：684 passed / 0 failed（+ 集成测试：session_event_log_e2e 3、memory_e2e 3 等）
```

### Rust（macOS，Apple Silicon）
```bash
brew install rust libsodium pkg-config   # 一次性；libsodium 由 pkg-config 自动发现
cargo check --manifest-path packages/app/src-tauri/Cargo.toml
cargo test --lib                          # build.rs 的 manifest 注入仅 MSVC 生效，mac 跳过
```

### 前端
```bash
pnpm run tauri:dev     # 开发模式（须在仓库根目录运行；端口 1420，被占时先 taskkill //F //PID <pid>）
pnpm run tauri:build   # 打包（须在仓库根目录运行；packages/app/ 下无此 script 会报 Missing script）
pnpm test          # vitest（本地可跑，前端重构主安全网）
pnpm typecheck && pnpm lint && pnpm build   # 不覆盖视觉/CSS 回归
```

## 架构概览

```
packages/app/src-tauri/src/
├── commands/         # Tauri 命令入口（chat/agent/conversation/mcp/kb/project/preferences/log/message）
├── harness/          # 核心业务逻辑
│   ├── mcp/          # MCP 工具系统（client trait/registry、内置工具、外部 server、bundled runtime、proposal_tool）
│   ├── provider/     # LLM provider 适配（anthropic/openai/mock + model_info 模型窗口表 + embedding）
│   ├── loop_engine.rs# 主循环调度（697 行，已拆出 loop/ 子模块）
│   ├── loop/         # 拆分出的子模块（context/events/reason/retry_round/stuck_detect/token_usage）
│   ├── tool_executor # 工具执行编排 + 授权流程
│   ├── proposal_guard.rs / proposal_registry.rs  # 配置提案 guardrail + 通道
│   ├── hooks.rs      # 对话钩子执行器（run_hooks + has_actions，4 接入点）
│   ├── kb/           # RAG 知识库（embedding/indexer/parser/watcher/ensure）
│   ├── budget.rs / summary_provider.rs / chat_state.rs / cleanup.rs / batch_writer.rs / oneshot_registry.rs / observable.rs
│   └── context/      # 上下文管道（token 估算、历史加载、摘要、裁剪阶段）
├── db/               # sqlx 数据层（models/repo/migrations）
├── infra/protocol.rs # 跨层事件 payload
└── lib.rs            # 启动入口（registry 初始化、MCP boot）
packages/app/src/
├── components/chat/  # 聊天 UI（ChatMessages, ConfigProposalCard, ToolAuthDialog...）
├── composables/      # 前端逻辑组合（useChatEvents, useThinkingTimer, useScrollFollow, useTheme, useNewConversation）
├── stores/           # Pinia 状态管理（chat, agent, project）
├── api/bridge.ts     # Tauri invoke 统一入口
└── types/index.ts    # 前端类型定义
```

## 关键系统

### 配置提案系统（Phase 1，已 commit a4f0e5f + push；agent.yaml 写保护加固 132cf19）
agent 调用 `propose_config_change` 工具提出创建/修改 agent 提案 → 前端渲染审批卡片 → 用户批准后前端走现有可信 Tauri 命令应用。**agent 全程无写权限**。
- `proposal_tool.rs`(mcp/) + `proposal_guard.rs` + `proposal_registry.rs`(harness/) + `ConfigProposalCard.vue`
- guardrail：🔴红线→Err，🟡→Medium，🟢→Low；API Key 走引用槽位（`key_slot:"__SLOT__"`），用户在卡片亲手填
- 安全加固(132cf19)：写工具 `reject_sensitive()` 拦硬写 agent.yaml + `register_meta_tools()` 强制注入合法通道

### MCP 工具系统
- `McpClient` trait：name/description/parameters/execute/execute_with_context
- `McpRegistry`：RwLock<HashMap<String, Arc<dyn McpClient>>>
- `McpServerManager`：统一 Server Pool 状态机（Disabled→Starting→Running/Failed），启动时后台并行启动
- 内置 MCP runtime：内置 Node + 预打包包（runtime_kind 列）；当前 2 包（builtin-thinking + builtin-memory，filesystem 已于 v0.2.5 下线）
- 文件工具 native 化：`file_tools.rs`（Write/Edit/Delete/Move/CreateDirectory），不再依赖外部 filesystem server

### Word 文档能力（0.5.0，docs/word-capability-roadmap.md 为路线真相源）
- 读侧 `inspect_docx`（mcp/docx_tool.rs + doc/docx_inspect.rs）：outline/format/text/headers_footers/ppr 五档投影 + start/end 区间；块编址 1-based 混排（sdt 摊平）= 编辑地址地基；有效格式三层合并（直接 > basedOn 链 > docDefaults，doc/styles.rs）+ numbering.xml 计数模拟（自动编号实际值，祖先级未现按该级自身 start 渲染）
- 编辑 `edit_docx`（doc/docx_edit.rs）：zip 手术引擎——只替换目标 XML 部件、其余 entry 字节原样重打包；operations 批量事务（全有或全无 + expect_prefix 地址指纹 + 模型级 diff 读回 + 备份/原子写）；六操作 replace_text / insert_paragraph_after / delete_block / set_format / set_style / set_ppr_element
- **D9 通用元素手术层**：pPr 子元素 ~34 封闭 schema 集（`PPR_ELEMENTS` 白名单兼当 schema 位插入序）→ set_ppr_element 一个操作永久收敛段落格式长尾；sectPr/pPrChange 受保护；片段校验（禁 xmlns/单根/根名与 element 一致/深度平衡）；摘段级 numPr 时样式链回退诚实警告
- **S3 四波·表格格式四件（D11，2026-08-25）**：读侧格式可见（模型 tblPr/trPr/tcPr 特征 + table 投影「表属性」摘要行与格级标注 + `projection=tblpr` 三级原文下钻：默认 tblPr / row→trPr / row+cell→tcPr）+ `set_table_element`（level=table/row/cell 通用元素手术，TBLPR 17 / TRPR 12 / TCPR 13 封闭白名单兼 schema 位插入序；gridSpan/hMerge/vMerge 受保护指路 merge_cells）+ `set_cell_format`（格级段落+字符格式，段落→格内全部段、字符→格内全部 run）+ `merge_cells`/`split_cell`（Word 原生语义：纵并 restart/continue 内容留原格、横并 gridSpan 求和内容拼首格；**网格列区间对齐判据**（同格号 ≠ 同网格列）；结构重构独占一批）。⚠️ 排障纪要：内层元素寻位（find_element_span 返回**切片内偏移**）勿拿去索引原串——须用切片变量索引（凑巧对齐时假绿，tcW 后跟 vMerge 即读垃圾）
- ⚠️ 不变式：编辑从 docx_model 类型树出发勿回字符串扫描；找元素验后随字符防前缀碰撞（`<w:pPr` 撞 pPrChange / `<w:b` 撞 bCs）；含活动修订（w:ins/w:del）文档编辑默认拒；AppliedOp 摘要按 splice 序非输入序（断言须次序无关）。🚫 语料保密（D7 硬禁令）：三份真机 docx 只留本地 tests/fixtures/docx/（gitignore），任何语料字符串（标题/正文词/样式名）零进代码/注释/文档，corpus 测试运行时读取+缺失 skip

### 对话钩子系统（已 commit 1c2a1d8 + push）
- 4 接入点：ConversationStart(chat_cmd) / BeforeLlm(loop_engine) / AfterTool(tool_executor) / ConversationEnd(loop_engine)
- 内置动作：InjectPrompt/CallTool/Log
- 配置在 agent.yaml `hooks` 字段（AgentFileConfig.hooks）

### Agent 质量拍·工具层不变式（2026-08-23，诊断驱动：826 次失败样本）
- **报错即行为契约**：工具错误文案三段式（发生了什么 + 为什么 + 怎么办），not-found 必挂 `mcp/path_suggest.rs::suggest_for_missing` 近似候选；禁止裸 io 错误出工具层
- **错误文案首行 = 稳定家族前缀**（家族词在前、路径在后，冒号分隔）——doom_loop 按「工具名+首行冒号前缀」算错误签名，路径混进前缀会把签名打散、连败检测失效
- write_file `create_dirs` 默认 true（好默认，用户拍板 2026-08-23）；run_command Windows 恒前置 `chcp 65001 >nul & `（中文输出统一 UTF-8，勿删）
- **doom_loop**（`loop/doom_detect.rs`，P10④）：同签名连败 3 次 nudge（tool_result 尾注纠正指令 + hook_injected point=doom_loop_nudge 入日志）/ 6 次终止（finish_reason=doom_loop 走 finalize_guard 对称清场）；与 stuck_detect 分工——前者抓「签名不变」，后者抓「轮指纹不变」（换文件名重试同类失败只有前者能看到）
- **⑤ system prompt 两层设计（2026-08-23，docs/agent-prompt-draft.md）**：平台层（`context/system_prompt.rs`）只放风格中立纪律（错误纪律/诚实边界/语言跟随；意图确认归工程档**不进平台层**——与创作档「动笔前先对齐」重复）；风格是人格，归 agent.yaml `system_prompt`，前端「风格预设」三档（`data/stylePresets.ts`）是**素材不是档位**——插入即用户文本，零版本纠缠。写 yaml 多行块走 `set_agent_system_prompt`（块级补丁+回读闸+原子写）；`set_agent_yaml_field` 是 u64 标量专用，勿混用。

### 上下文预算（Phase 0+1+2，已 commit push）
- 真实 token 估算（覆盖 tool_use/tool_result/thinking/image 块）+ per-agent context_window
- TokenWindowStage（max_input_tokens 的 80% 硬裁历史）
- Phase 2 滚动增量摘要（covered_until_rowid 追踪 + fold 55%·40%）

### RAG 知识库（已 commit push）
- KB 文档 → embedding → 索引 → 语义检索（search_kb 工具）；watcher 自动索引；产品帮助种子已落地

### 工具名合规化（deepseek 400 双修，commit a02a7b0，已 push origin/main）
- migration 39 `tool_index` 列 + `t{idx}_` 命名 + 历史 sanitize（修工具名违反 `^[a-zA-Z0-9_-]+$`）
- OpenAI 适配层 `chat_message_to_openai` 1→N 展开 tool_result 为多条 role=tool（OpenAI-only，Anthropic 零改）

### 大文件拆分（已 ff-merge 到 main 17b1ffc，分支已删）
- loop_engine 1343→697 + 抽 `loop/` 子模块；chat.ts 843→532（抽 useChatEvents）；Sidebar/ChatMessages 抽 composables

### 视觉能力统一适配（事2 / 方案 C，bfcd2ce + 2ce76cb + f054e38 + c10d02e，未手测）
4 个 Image 块注入入口统一走"按有效视觉能力适配"，杜绝向非视觉模型塞 Image（→400/"看不到"）：
- **能力探测**：`provider/model_info.rs::effective_supports_vision(agent.supports_vision, provider, model)`——OR 关系（agent 显式 =1 权威；=0 按模型表自动探测，如 MiniMax-M3）。零 schema 改动。
- **统一适配**：`harness/modal.rs`——`gather_vision_candidates`（DB 收集凭据：显式 vision 配置→agent 自带视觉模型→GLM 视觉 MCP env）/ `adapt_blocks_for_vision`（有效视觉原样过；非视觉逐图代读成 Text、失败剥离+诚实提示）/ `strip_image_blocks_to_marker`（历史静默剥离）。
- **4 入口接线**：① 用户上传+③ 历史 → `context/stages.rs::ModalCapabilityStage`（Pipeline，TokenWindow 后 Final 前）；② 工具返图 → `tool_executor` 注入 Image 前查 effective_vision（时序独立于 Stage——工具循环后续轮次的图进不了 Stage，必须在此守卫）；④ `view_attachment_image` 判断改 effective_supports_vision + 凭据收集复用 gather。
- **⚠️ 不变式**：任何新增的 Image 块注入点都必须经 `effective_supports_vision` / `adapt_blocks_for_vision`，不得对非视觉模型直塞 Image。

### 会话事件日志（Phase 0+1+2A 已发布；Phase 2B 退役三件套已落地 2026-08-17）
单一 append-only 事件日志基石（锁定愿景：统一 session / 多 agent 图协作 / 轨迹可还原）。
- **表**：migration 44 `session_events`（seq INSERT 子查询原子 + UNIQUE 兜底；message_id 故意无 FK——事件须活得比被删占位行久）
- **词表 13 kind** + typed emitters：`harness/event_log.rs`（EventCtx + warn-only 影子定位）
- **接线全退出路径**：chat_cmd/memory/loop_engine/cleanup(PersistOutcome)/retry_round/tool_executor/stages；**硬规则：事件 inline `.await` 禁 spawn，turn_ended 必须先于 cleanup() unregister 落库**
- **supersede**：自动续写同 message_id 多条 assistant_message，回放 last-wins
- **导出**：`export_session_trajectory` 命令 → JSONL（docs/backend-api-reference.md）
- **Phase 1 对账（889e9a8..0baa06c，6 commits）**：`harness/derive.rs` 纯回放（supersede last-wins / 空回退对称 / 坏 payload 记 issue 不吞）+ `harness/reconcile.rs`（A 侧 legacy 行提取走 `list_all_by_rowid` rowid 全量序 + 同一 `parse_content_blocks`；B 侧事件回放；turn 锚点走查分组）。diff 五类 MISSING_IN_DERIVED/MISSING_IN_LEGACY/CONTENT_MISMATCH/ORDER_MISMATCH/DERIVE_ISSUE = bug 清单；skipped 全部已文档化容忍（pre_phase0/epoch/incomplete_turn/error_row/discarded_row/empty_placeholder）。`reconcile_session` 命令只读出口。**真机验证：9a2a1968（20 事件 9 行）diffs=[] 且 skipped=[]；eae6d983（36 行零事件）全落 pre_phase0_no_events**。⚠️ 不变式：turn_id == user_msg_id；对账平面 = 行级原始形态（不跑 sanitize/投影）
- **Phase 2A 读路径切换（已随 0.3.6 发布）**：事件日志从影子升格为**干净会话的主读路径**。`harness/read_route.rs` 按会话路由：有事件 + 对账零 diff + 纯事件纪元 → **Derive**（`load_history_from_events` 派生 `Vec<MessageRow>`，锚回真 rowid，走与 legacy 完全相同的下游 Pipeline；派生输出与 legacy 同构同函数，reconcile 已证逐字节相等；摘要 `source_rowid` 取真 rowid 保连续性）。**指纹缓存** `(max_seq, max_rowid)` 追踪新数据（每轮刷新）；原地篡改不被察觉但活跃会话下轮即刷新、休眠会话不被读——`reconcile_session` 命令始终新鲜。诊断 `get_read_route_status` 命令。**不变式**：派生 MessageRow 必须能过 `load_history_with_window` 产出与 legacy 完全相同的视图（含 source_rowid）。
- **Phase 2B legacy 拼装退役（S1 阶段 1，2026-08-17）**：`load_history_from_events` 成为**唯一**生产读路径（session_runner 恒走派生）；`resolve()` 降级为健康监控——非绿（no_events / reconcile_diffs / mixed_epoch）时 error 日志后**照常派生**（历史可能缺行，不再静默回退 legacy；写路径 bug 不再被自动兜底静默吞掉）。排查走 `reconcile_session` / `get_read_route_status`；偏好回滚开关随分支删除（`session_read_path` 不在 KNOWN_KEYS，库内残留无害）。**回滚 = revert 阶段 1 commit**（messages 表双写持续，Legacy 拼装可整体恢复，零数据损失）。零事件会话行为锁定（e2e 场景 7）：派生空历史 + 回合照常完成 + boot backfill 自愈。
- **Phase 2B 前置 backfill（已落地 2026-08-17，3 commits，未 push）**：`harness/backfill.rs` boot 幂等扫尾——给零事件旧会话反向合成事件（reconcile 的逆函数：同 `parse_content_blocks`/空回退对称/同容忍清单 → 构造性零 diff → read_route 现有判据自动放行 Derive，read_route/derive/reconcile **零改动**）。范围只补零事件会话（混合纪元不补——seq 追加语义装不进历史前缀）；`turn_context` 不合成（payload 要 provider/model 快照，旧行没有，填当前配置=伪造）。actor=`backfill` 行是派生数据非运行时事实（append-only 边界的显式例外，重跑可删）；termination=`backfill` 诚实标注；created_at 直传行时间戳。**版本化重跑**：BACKFILL_VERSION 存 preferences，代码>库内 → 纯 backfill 会话删旧重写自愈；**冻结规则**：混入真实事件后永不可重写（会错序），frozen 仅计数。绕过 append_event 走 repo 批量（显式 seq/单会话事务/不广播）。7 测试：全形态零 diff+Derive green / 视图等价 / 幂等 / 混合纪元不碰 / 孤儿行降级 Legacy / 版本重跑 / 冻结。待真机验收：boot 日志 `[ice_paw.backfill]` 行 + 旧会话 `get_read_route_status` 变 Derive。
- **Phase 2B 阶段 2 摘要锚点 seq 化（2026-08-17）**：migration 46 `covered_until_seq`（= 被覆盖消息首现事件 seq，与 derive 排序位严格一致）+ 存量回填；`SummaryState`/insert/update/SELECT 双写双读；`ChatMessage.source_seq`（`#[serde(skip)]`，不进 LLM payload）；锚点定位 seq 优先 `.or_else` rowid 兜底；`SummaryPayload.covered_until_seq`（`#[serde(default)]`，旧事件零迁移）。显式双写过渡，回滚干净（列闲置无害）。
- **Phase 2B 阶段 3 Image 双份存储治理（2026-08-17，3a 读侧 + 3b 写侧）**：消息类 payload 的 blocks 用 `PayloadBlock` untagged 双形态——`Full(ContentBlock)`（v1 内联，旧事件零迁移可读）/ `ImageRef{message_id, block_index}`（v2，字节只在 messages 行）。写侧唯一入口 `refify_blocks`（emitter 字段式签名内部做，调用方传与落库同值的 blocks）；读侧三路水合：derive `hydrate_image_refs`（纯同步 resolver 注入；未命中/越界/非 Image 降级 `Text("[图片内容已不可恢复]")`）+ `to_content_blocks` 防泄漏最后闸 + conversation_cmd JSON 级水合（list_session_events/export，前端零改动）。BACKFILL_VERSION=2（纯 backfill 会话删旧重写自愈，冻结会话保留 v1 照读）。**⚠️ 不变式：session_events 消息类 payload 禁止内联 Image base64——新增 message-kind emitter 必须经 `refify_blocks`，读侧必须经 `hydrate_image_refs` 水合后才能进对账/LLM 视图（ref 形态不得以非 Text 形态流出）**。

## 当前状态（2026-08-26）
- 版本 **0.5.2 已打包**（= 0.5.1 + **S3 四波·表格格式四件**[读侧格式可见 tblpr 三级投影 / set_table_element / set_cell_format / merge_cells·split_cell] + **S3 五波·样式档案与模板个性化**[def_edit 三操作 create_style·set_style_element·set_numbering_element + styles/styledef/numbering 三投影 + table_style·clear_body 顺路件 + 分族路由 + word_style_profile 双轨承载：yaml 块→prompt 注入 hooks 旁路 + set_agent_word_profile 命令 + 提案通道 🟢 Low ""=摘除 + 前端薄接线]；cargo 1122）——生产驱动：三波补了表格内容但边框/底纹/字体/样式无工具；用户需求「工具层通用抽象 + 不同用户不同模板偏好」
- 上一版 **0.5.1**（三波·表格内容四件；cargo 1071）；再上 **0.5.0**（Word 能力演进整线 + Agent 质量拍 Phase 1；cargo 1060 / vitest 326）
- 分支：仅 `main`
- 近期递进：0.4.1 → 质量拍 Phase 1 + Word 能力演进整线（S0a→S0b→手术引擎→S3 首波→真机复盘两批→D9 set_ppr_element）→ 0.5.0 发版 → 生产实战反馈表格双缺口 → S3 三波表格四件（D10）→ 0.5.1 打包 → 生产反馈表格格式缺口 → 四波（D11）→ 样式通用抽象+个性化需求 → 五波（D12 双轨承载）→ **0.5.2 打包**
- `cargo test --lib` 1122 passed / 0 failed（+ 集成测试：session_runner_e2e 7、session_reconcile_e2e 6+2 ignored、session_event_log_e2e 3、memory_e2e 3、message_repo 7、provider 11）；clippy --tests -D warnings 0 警告；vitest 326
- 仍待办：**0.5.2 真机手测**（三波表格内容 + 四波表格格式：Word 打开验收[尤其合并格表/横并拆分] + projection=table/tblpr 寻址准确度；五波：样式定义改写后投影三层合并显示新值/自动编号不变 + word_style_profile 全链路[口头偏好→审批卡→yaml 落块→下回合 prompt 生效]；另有 0.5.0 遗留：set_format/set_style/set_ppr_element Word 打开验收、DB 诊断复跑 5.7%→<2%、风格预设手测；WPS 样本仍缺）、视觉适配/KB watcher/自动续写生产手测、proposal Phase 2（MCP 域）、V5 钩子未用未测、Word 后续波（TOC/图片/条件批量替换）
- **预算诚实化不变式（0.3.9）**：新 provider usage 必须归一规范语义（prompt=总输入含命中、cached≤prompt；Anthropic 显式归一 + stream_consumer `into_canonical` 自愈兜底）；工具列表出口恒按名序（前缀缓存前提，勿回退）；DeepSeek 私有对优先于标准字段
- **S1 真机验收 2026-08-17 四项绿**：backfill（sessions=9 events=824 failed=0 epoch_rows=0，版本标记=2）+ 恒 Derive（当日路由决策全 green diffs=0，含 backfill 会话续聊 seq 1..933 连续）+ 发图 v2 payload 无 base64（image_ref 162B 指针，本体 851KB/3.8MB 只在 messages 行；模型回复描述画面=水合进 LLM 视图实证）+ 摘要折叠 `covered_until_seq=726`/rowid=1710 双值落库

## 关键不变式：60s 静默超时——心跳（快速路径）+ 后端真相确认（最终裁决）

**问题**：前端 60s 静默超时（`stores/chat.ts:resetSendTimeout`）假定「后端必有活动事件回报」，但 send→done 之间横跨 Pipeline / 多图 OCR / MemoryStage 摘要 / 单个慢工具（MCP 上限 120s）/ 慢 TTFT 等串行重活，**逐点埋心跳枚举永远不全**。误判 `sending=false` 的后果是级联的：第二条消息打进还在跑的回合（后端 `chat_state.start` 拒绝 → 前端只 console.error，用户看「发送没反应」）+ 乐观用户消息从未落库（切走再切回凭空消失）。

**根治（两层）**：
1. **心跳快速路径**：后端在已知重活步骤 emit `chat:processing`（Pipeline 入口/出口 + OCR 每张图），前端收到即 `resetSendTimeout()`。
2. **后端真相确认（最终裁决，2026-08-22）**：60s 超时触发时前端**不直接**翻转 sending，先 invoke `is_conversation_streaming` 问 ChatState 注册表——仍在跑就重新计时，确认死了才翻转。这让「未知静默窗口」在结构上不可能引发误判，新增长耗时路径无需埋点自动被兜住。后端注册表（`chat_state.start` 注册 / cleanup unregister）是唯一真相源。

**不变式**（守住，勿删勿改）：
1. **chat:processing 不进 session-event-log**：心跳不是业务事实，是瞬态 UI 信号——否则日志会爆。走 LoopEmitter 通道，与 session-event-log 分工固定。
2. **稳定 stage 词表**：`pipeline | ocr`（前端 i18n 用）——别新增临时词条，每加一条要让前端 i18n 一起更新。
3. **OCR 每张图完成（成功或失败都算 done）emit 一次**：通过 `adapt_blocks_for_vision` 第 4 参 `Option<ProgressCb>` 注入；视觉模型分支（早返回）不触发回调。
4. **前端 chat:processing handler 只做 `resetSendTimeout()`**：别加 UI 文案 / 别加 store 状态——事件频率不可控（每张图 1 次），状态写入会让 store 频繁抖动；UI 文案由未来"进度条 UX"任务独立做。
5. **60s 超时常量不动**：超时只触发「问后端」，不是翻转；改阈值会掩盖"心跳不够密"的真实问题。
6. **超时确认的会话归属用 `sendingConvId`**（发起回合的会话），勿用 `activeConvId`——用户切走后 activeConvId 已变，问错会话必误判。
7. **发送失败必须可见 + 回滚**：`sendMessage` catch 写 per-conv 错误横幅（含重试，`lastFailedSend` 已备）+ 移除乐观用户消息（后端已拒，未落库的气泡切回会消失）。并发拒绝（「在途生成任务」）走专属文案。

**验收**：10 张图串行 OCR（每张 8s 网络延迟，总 80s）期间，前端不触发 `sending=false`，切走/切回 ChatPage 时 streaming 状态保持；chat:done 到达时正常进入完成态。人为制造 >60s 静默（如断点暂停后端）时，超时轮询确认后端仍在跑 → 不误判；回合进行中再发消息 → 横幅提示 + 气泡回滚。

**关键文件**：
- 协议：`infra/protocol/events.rs:ChatProcessingPayload`（stage + message + 可选 progress）
- 发射：`harness/modal.rs:adapt_blocks_for_vision` 第 4 参 `ProgressCb` + `context/stages.rs:ModalCapabilityStage` 注入 emit 闭包 + `harness/session_runner.rs` Pipeline 入口/出口
- 接收：`composables/useChatEvents.ts:chat:processing` → `chat.resetSendTimeout()`（勿加会话过滤）
- 真相确认：`commands/chat_cmd.rs:is_conversation_streaming` ← `harness/chat_state.rs` 注册表；前端 `stores/chat.ts:resetSendTimeout`（超时→轮询→确认才翻转）+ `api/bridge.ts:chat.isStreaming`
- 测试：`harness/modal.rs:progress_cb_*` 三档（无图 0 触发 / 视觉直通 0 触发 / total 等于 Image 块数）

