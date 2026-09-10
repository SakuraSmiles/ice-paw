# CLAUDE.md — ice-paw 项目指引

## 项目概述
IcePaw — 本地优先的 LLM 对话工作站。Tauri v2 (Rust) + Vue 3 (TypeScript) 桌面应用。
当前版本：`0.6.16`。

## 设计规则（用户拍板，勿翻案）

**配置放置阶梯**——想新增任何配置项，先按序过四层，落不进 L1-L3 才考虑 L4：
- **L1 好默认**：能靠默认值 + 自适应解决的（如预算 3× 窗口、摘要自适应额度），根本不成为配置
- **L2 状态上屏**：用户需要「看见」的做成 HUD/胶囊/toast（如预算 pill），不是可编辑字段
- **L3 agent.yaml**：专家旋钮进 yaml +「打开 agent.yaml」入口 + 配置提案通道（propose_config_change / set_agent_yaml_field）
- **L4 表单**：只放出生证（name / id / model / key / workspace）

一句话：**状态上屏，配置进 yaml，表单只管出生证**。透明度问题（看不见）≠ 可配置性问题（不能改），勿用加表单字段治看不见——此为用户多次纠正的系统性偏差（最近一次 2026-08-16，P7 回滚 AgentForm 高级区）。

**视觉规范三则**（2026-08-20 表现层走查后拍板）：

1. **色彩三层架构**——交互层藏青单色（浅 #1E4976 / 深 #4E80C0，双锚点十一档）+ 语义层三色（danger/success/warning，已建勿动）+ 身份层点缀（--ip-accent-agent 紫等）。info 并入主色（信息提示=品牌蓝，不新增第五色）。品牌色单一真相源在 tokens.css，global.css 不得再定义 --ip-primary-*。tokens 三主题区（:root 浅 / data-theme 暗 / prefers-scheme 自动暗）改动必须三区同步——2026-08-20 实测：单区插入静默失败致暗色聊天区整体回落浅色值。
2. **图标规则（系统级零 emoji）**——一律 Lucide（npm 包名 `@lucide/vue`，非 lucide-vue-next——2026-09-01 探查勘误，仓内 7 文件在用），24×24 stroke 线性语言、currentColor 单色；禁止 emoji 出现在 UI 任何位置（含按钮/标签/占位文案）。语义档位（原 🟢🟡🔴）用「语义色圆点 + 文字」或 Lucide shield 系图标。例外区仅两个：ProviderIcon 品牌 glyph（simple-icons 填充式，集中管理）+ EntityAvatar 哈希色板（数据色板）。存量 105 个手写 SVG 渐进替换，新代码一律 Lucide。
3. **字号与间距**——字号走 --ip-text-* 九档梯度（display40/h1-28/h2-22/h3-18/body-lg-17/body-15/body-sm-13/caption-12/**micro-11**/code-14）；micro 是最小合法档（~86 处幽灵字号已于 UI-4 批次 A 收编）；**9px/10px 是可读性黑洞，禁止出现**，新代码小字一律 micro。间距：布局级（页面 padding/卡片内边距/区块 gap/列表行距）走 --ip-spacing-* 令牌，光学微调位（图标间隙 2-4px）允许字面量；新代码布局间距强制令牌。字体家族一律 --ip-font-sans/mono/display，勿直写字体名。

4. **字体本地化（产品哲学）**——IcePaw 是本地优先产品，**禁止任何网络字体加载**（Google Fonts/CDN 字体一律不用）；字体经 @fontsource npm 包或 assets/fonts 自托管 woff2 打进安装包。中文字体注意子集化（只带实际用到的字重）。首启离线环境的观感是验收线。

5. **z-index 阶梯令牌化**——层叠层级一律 var(--ip-z-*)（base/dropdown/popover/modal/toast 等 tokens 已定义），禁止裸数字 z-index。浮层嵌套时用「相对层级」思路（父层提高，子层跟父），不做 9999 军备竞赛。

6. **加载态三档规范**——按场景选型：**首屏/首载 → 骨架屏**（shimmer，侧栏样式为模板）；**列表刷新 → 顶部细进度条**（不顶开内容）；**操作等待 → 按钮内 spinner/文案变化**（如"保存中…"）。禁止全屏遮罩 loading（除模态流程必须阻塞时）。

7. **图片内容规范**——气泡附件图：max-width 限定 + 圆角 --ip-radius-lg + loading=lazy + 点击进预览器；头像：EntityAvatar 三级降级链（勿绕过）；图片预览器须支持键盘 ←→ 翻张/Esc 关闭；加载失败用中性占位块 + "图片不可用"文案，不留破图图标。

8. **文案规范（ux writing）**——直接、克制、不感叹、不用语气词（哦/啦/呢）；对用户称"你"不用"您"；错误文案三段式：**发生了什么 + 为什么 + 怎么办**（error_mapping 已是此形状，前端展示对齐）；域术语固定写法：会话/Agent（大写 A）/委派/轨迹/预算/知识库（KB 首次出现标注）；数字与单位之间空格（"3.2 MB"、"214 ms"）；时间一律相对时（timeAgo）+ hover 绝对时。

9. **无障碍基线**——键盘可达所有操作（Tab/Enter/Esc/方向键，弹层焦点管理）；:focus-visible 全局焦点环（组件不得用 outline:none 抵消而不补替代）；文本对比度 AA（4.5:1，大字 3:1）；reduced-motion 全局降级已有（tokens §12），新增动效必须跟随。

10. **编辑交互契约（2026-09-08 拍板：全系统统一显式保存）**——任何编辑入口一律草稿态 + 显式「保存」（成功收起/「已保存」淡出）+「取消」回滚；禁止 blur 即存 / 选择即存（历史即时保存位为迁移负债：通用页各卡、会话标题行内改名、项目成员/项目背景——迁一处删一处，新代码零新增）。跨字段校验（换厂商 key 闸等）= 保存时拦截（模型页 deferred「待补齐」半成品机制已随之退役——表单模式下不再需要）。**不可恢复删除一律两步确认**：第一次点击武装成 danger 确认键、第二次执行、外点/blur 解除；载体随语境（菜单内确认条 / 行内横向条 / 按钮武装态，MoreMenu/ChatHeader/模型页三处已互相对齐；项目级联删除带数据处置选择的 Modal 属合理重载体，不压平）。创建流程本就是显式提交，不受影响。

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
├── commands/         # Tauri 命令入口（chat/agent/agent_yaml/conversation/mcp/kb/project/preferences/log/
│                     #   message/provider/model_profile/screen/inbox）
├── harness/          # 核心业务逻辑
│   ├── mcp/          # MCP 工具系统（client trait/registry、内置工具 file_tools/shell/docx_tool/screen/...、
│   │                 #   外部 server、bundled runtime、proposal_tool/delegate/plan_tool/relay）
│   ├── provider/     # LLM provider 适配（anthropic/openai/mock + model_info 模型窗口表 + embedding）
│   ├── doc/          # Word 文档子系统（inspect 读侧投影 / edit zip 手术引擎 / styles / numbering / assets 共享模板）
│   ├── loop_engine.rs# 主循环调度（已拆出 loop/ 子模块；膨胀观察：697→1377 行）
│   ├── loop/         # 拆分出的子模块（context/events/reason/retry_round/stuck_detect/token_usage/
│   │                 #   fallback 降级链换档）
│   ├── tool_executor # 工具执行编排 + 授权流程
│   ├── proposal_guard.rs / proposal_registry.rs  # 配置提案 guardrail + 通道
│   ├── profile_match.rs / profile_materialize.rs / profile_health.rs
│   │                 # ModelProfile 实体：去重键单一真相源 / 新建自动物化 / 健康三列
│   ├── agent_profile_migration.rs / legacy_model_migration.rs
│   │                 # boot 存量抽离（幂等标记）/ legacy Key 收编 Stronghold
│   ├── session_runner.rs  # 回合编排（Pipeline 接线、LoopContext 构造唯一生产点）
│   ├── event_log.rs / derive.rs / reconcile.rs / read_route.rs / backfill.rs
│   │                 # 会话事件日志五件（typed emitters / 纯回放 / 对账 / 读路由 / boot 扫尾）
│   ├── hooks.rs      # 对话钩子执行器（run_hooks + has_actions，4 接入点）
│   ├── inbox.rs      # MA-3 跨会话通讯（投递/收件三态/消费引擎/护栏）
│   ├── kb/           # RAG 知识库（embedding/indexer/parser/watcher/ensure/vector_cache）
│   ├── modal.rs / vision.rs  # 视觉两档制（能力探测 / 代读适配 / 条目链）
│   └── budget.rs / summary_provider.rs / chat_state.rs / cleanup.rs / batch_writer.rs / oneshot_registry.rs / observable.rs
├── context/          # 上下文管道（stages Pipeline：token 估算/历史加载/摘要/TokenWindow/ModalCapability/ScreenshotHistory）
├── db/               # sqlx 数据层（models/repo/migrations）
├── infra/            # protocol/（跨层事件 payload）+ strings/decode/file_validation/path_norm 等
└── lib.rs            # 启动入口（registry 初始化、MCP boot、WebView2 缓存瘦身）
packages/app/src/
├── components/       # 组件（chat/agent/common/kb/layout/mcp/project/trajectory 八子域）
├── composables/      # 前端逻辑组合（useChatEvents/useModelProfiles/useProjectTrajectory 等 16 个）
├── stores/           # Pinia 状态管理（chat, agent, project, screenChannel）
├── pages/            # 路由页（settings/ 含各设置页、project/、screen/）
├── router/           # 路由表
├── data/             # 前端常量（stylePresets 等）
├── utils/            # 工具函数（toolSummary/toolLabels/format 等）
├── api/bridge.ts     # Tauri invoke 统一入口
└── types/index.ts    # 前端类型定义
```

## 关键系统

### 配置提案系统（Phase 1，已 commit a4f0e5f + push；agent.yaml 写保护加固 132cf19）
agent 调用 `propose_config_change` 工具提出创建/修改 agent 提案 → 前端渲染审批卡片 → 用户批准后前端走现有可信 Tauri 命令应用。**agent 全程无写权限**。
- `proposal_tool.rs`(mcp/) + `proposal_guard.rs` + `proposal_registry.rs`(harness/) + `ConfigProposalCard.vue`
- guardrail：🔴红线→Err，🟡→Medium，🟢→Low；API Key 走引用槽位（`key_slot:"__SLOT__"`），用户在卡片亲手填
- 安全加固(132cf19)：写工具 `reject_sensitive()` 拦硬写 agent.yaml + `register_meta_tools()` 强制注入合法通道
- **旋钮唯一写入通道不变式（2026-08-27 ②-1）**：旋钮字段（system_prompt / temperature / max_tokens / max_total_tokens / tool_max_rounds / enabled_tools / word_style_profile / hooks）的写入通道 = yaml 通道命令（`set_agent_system_prompt` / `set_agent_yaml_field`[整数族+temperature 浮点] / `set_agent_enabled_tools` / `set_agent_word_profile`），`update_agent` 只管出生证字段（name/provider/model/base_url/workspace/avatar/key）。出生 yaml 的活行遮蔽 DB 列（apply_to_row），写 DB = 「批准后生效」假象。ConfigProposalCard 批准路径按字段分派（编辑值 editFields 覆盖优先）；`base_url` 双层 Option：显式 Some=设/清、absent=保持（仅换厂商且有默认时跟随，`resolve_base_url_arg`——回归测试在 agent_cmd tests）
- **⚠️ enabled_tools 镜像同步不变式（2026-08-31 生产实案）**：工具组装期收窄读的是 **DB 行**（yaml 仅加载时经 apply_to_row 遮蔽、字段 Some 才覆盖）——`set_agent_enabled_tools` 写完 yaml 必须镜像 DB 列（摘除→Some(None)=NULL、收窄→Some(Some)=同值 JSON），只摘 yaml 不清 DB = 旧白名单下次加载复活（工具静默缺失，0.6.1 真机两轮排查的根因）。组装期收窄生效时 session_runner 打 info 日志披露「保留 N/裁掉 M+名单」（治看不见，勿删）；生产排查线索：`enabled_tools: []` yaml 行 = 旧白名单阴影下的全开 workaround，根因修复后可摘

### 模型配置实体化 · ModelProfile（0.6.11~0.6.15 三阶段，随 0.6.15 发版）
模型配置从使用点散落抽成独立实体（`model_profiles` 表 + Stronghold 槽位 `profile:{id}`）：设置-模型页管理，视觉读取/语义检索/agent 对话统一引用——**Key 一处换全局生效**。
- **四列快照制**：agent 行 `model_profile_id` NULL = legacy 路径零迁移义务；非 NULL 时 `get_with_credentials`（agent_cmd.rs）解析 profile 覆盖出参 provider/model/base_url/api_key，**值变才回写快照列**（值同零 UPDATE，repo 的 IS NOT NULL 安全比较）；前端 6 显示位读快照列自动正确。⚠️ 提案 update_agent 对引用态 agent 手填快照族字段会被下轮解析覆盖（批准假象）——update 守卫须拦（2026-09-09 批次 R1）。
- **去重键单一真相源**：`profile_match.rs` GroupKey = 厂商+模型+归一端点+Key（trim/去尾斜杠/注册表默认合并）；新建自动物化（`profile_materialize.rs`）与 boot 存量抽离（`agent_profile_migration.rs`）同源——同配置复用既有实体不重复建。⚠️ 物化闭包三泛型直传勿收拢（rustc HRTB bug）。
- **键数锁**：删除守卫三段式拒绝，查两条腿——`agents.model_profile_id = ?` OR `fallback_profile_ids LIKE '%"id"%'`（带引号防前缀碰撞）+ prefs 视觉/语义检索腿。
- **降级链**（`loop/fallback.rs`）：`fallback_profile_ids` 有序多档，三拦截点——Quota 类不可重试分支（switch 先于 emit_round_error，**换档成功零错误终态**）/ RateLimited·Network 退避耗尽 loop-top / 分类表 `fallback_trigger`（Auth/Forbidden/ContextTooLong/Sensitive/Unknown 不换档）；换档重置 RetryState 但**预算 cap 不重置**；链尽走原终态；换档事件 `model_switch` kind + `chat:model-switched` toast。FallbackResolver trait 保 loop 链 Tauri-free（e2e 用 MapResolver）。**委派子会话同权**（0.7 批 A，`delegation_fallback_plan`——子 agent 自链优先，无链继承父 agent 配置链并摘除子主档同 id 档；父行读失败降级无链不阻塞委派；继承的是静态配置链非父会话换档剩余链，子会话 cursor=0 重新起跑）。
- **boot 迁移次序铁律**：抽离标记先落再扫（跑过零扫描）；手动形态豁免与 `manual_materializable` 同构（需 Key 厂商无 Key / custom 缺端点 → 旁路 legacy 不硬造 keyless 实体）。
- **健康归因**：`profile_health.rs` 三列（状态/最后错误/最后调用时间）；测试连接 `demote_unchanged_inputs`——入参与存量等值降级为 None 恢复归因（治「测试了但状态点不更新」）；链路测试 `test_agent_model_chain` 逐档**真发** 16 token 摘要请求（列模型是鉴权层动作验不出对话权益，Coding 1113 实案）+ Skipped（配置已删）刻意不记健康。
- **前端形态**：新建 agent 保留手写表单（保存时后端物化，UI 不感知）；编辑只允许选实体——合并 tag 链选择器（vuedraggable + 键盘换位，首 tag=主档 badge，chainIds[0]=主/slice(1)=降级）；悬空引用诚实标「（配置已删除）」。⚠️ jsdom 测不了原生 DnD（顺序语义靠键盘用例覆盖）；`.conn-btn` 双位须作用域定位。

### MCP 工具系统
- `McpClient` trait：name/description/parameters/execute/execute_with_context
- `McpRegistry`：RwLock<HashMap<String, Arc<dyn McpClient>>>
- `McpServerManager`：统一 Server Pool 状态机（Disabled→Starting→Running/Failed），启动时后台并行启动
- 内置 MCP runtime：内置 Node + 预打包包（runtime_kind 列）；当前 2 包（builtin-thinking + builtin-memory，filesystem 已于 v0.2.5 下线）
- 文件工具 native 化：`file_tools.rs`（Write/Edit/Delete/Move/CreateDirectory），不再依赖外部 filesystem server

### 屏幕读写 · Computer Use（0.6.0，docs/computer-use-roadmap.md 为路线真相源）
- **工具十一件**（`harness/mcp/screen/{mod,backend,coords,state,input,keyboard,channel,session,human,hud}.rs`）：看屏 capture_screen / capture_window / list_windows + 操作 mouse_move / mouse_click / mouse_drag / mouse_scroll / type_text / press_key + 节奏件 wait + request_screen_session（Confirm 级，不在 SCREEN_TOOLS 集合防死循环）；ScreenBackend trait 注入（Gdi/Unsupported/Fake）
- **坐标契约**：模型传的一切坐标 = 本会话**最近一次截图的图片像素空间**（ScreenState conv→CaptureMeta 64 LRU）；输入前三步换算 img→phys→abs（coords.rs 纯数学）+ 布局 revalidate（virtual_screen 变了 → 拒）
- **act-and-look**：六件操作成功后等画面稳定（隔 250ms 两帧逐字节相同才收敛，上限 2s）附「操作效果图」进同一 tool_result + **即刻写为新坐标基准**（消过期坐标 + 省一整轮确认）；**wait 永不附图**（Always 级授权治理）
- **屏幕共享通道**（授权与可见性的单位，治逐次审批卡）：会话头开关（唯一入口——暂停/终止住 HUD）或 agent request_screen_session → Confirm 一次 → 屏幕家族授权短路；关闭即全停，不跨重启持久化。治理三层：**单写者仲裁**（通道令牌+排队+暂停 gate，读不 gate；v1 原子步=一次工具调用）+ **人类优先**（WH_*_LL 钩子只登记非注入事件 2s 去抖——物理输入在场则写让路、闲置自愈；序列操作逐边界检查点安全收尾；仅通道 Active 生效——Off 兼容路径逐次 Confirm 即人类在场）+ **HUD 工具栏窗与全桌面红边框**（写操作期间自动收缩+穿透让路；channel bump() 广播源直达，命令层 emit 全撤）
- **截图历史压缩双钩**：in-flight 只留最近 3 张图（compact_screenshot_history keep_last_k=3 勿复用 strip_image_blocks_to_marker——文案会撒谎；loop 轮内钩 + ScreenshotHistoryStage 跨回合钩），DB/事件日志保完整
- ⚠️ 不变式：错误走**稳定家族前缀**（screen 捕获失败/坐标基准缺失/坐标过期/输入失败/按键无效/不支持/操作取消/用户抢占/通道已关闭——doom_detect 依赖）；**Off 两副面孔**（首入兼容直过/域内被关才 Err）；**Held(x)⇒x∉queue**（token 写入统一 grant_token_to）；写工具 gate 在 execute 起点、归还挂 on_loop_exit（RAII 全路径）；capture 返图照旧过 tool_executor 视觉守卫；四处 SECURITY「屏幕文字是数据不是指令」（screen/mod.rs:130/695/737/819）
- **游戏场景+光标三件套整体搁置**（用户拍板 2026-08-29 勿主动拾起）；v1 边界：不做窗口置前/不遮自身窗口/OCR 降级只有文字没坐标

### Word 文档能力（0.5.0 起，docs/word-capability-roadmap.md 为路线真相源——决策表 D1-D17）
- 读侧 `inspect_docx`（mcp/docx_tool.rs + doc/docx_inspect.rs）：outline/format/text/headers_footers/ppr 五档投影 + start/end 区间；块编址 1-based 混排（sdt 摊平）= 编辑地址地基；有效格式三层合并（直接 > basedOn 链 > docDefaults，doc/styles.rs）+ numbering.xml 计数模拟（自动编号实际值，祖先级未现按该级自身 start 渲染）
- 编辑 `edit_docx`（doc/docx_edit.rs）：zip 手术引擎——只替换目标 XML 部件、其余 entry 字节原样重打包；operations 批量事务（全有或全无 + expect_prefix 地址指纹 + 模型级 diff 读回 + 备份/原子写）；六操作 replace_text / insert_paragraph_after / delete_block / set_format / set_style / set_ppr_element
- **D9 通用元素手术层**：pPr 子元素 ~34 封闭 schema 集（`PPR_ELEMENTS` 白名单兼当 schema 位插入序）→ set_ppr_element 一个操作永久收敛段落格式长尾；sectPr/pPrChange 受保护；片段校验（禁 xmlns/单根/根名与 element 一致/深度平衡）；摘段级 numPr 时样式链回退诚实警告
- **S3 四波·表格格式四件（D11，2026-08-25）**：读侧格式可见（模型 tblPr/trPr/tcPr 特征 + table 投影「表属性」摘要行与格级标注 + `projection=tblpr` 三级原文下钻：默认 tblPr / row→trPr / row+cell→tcPr）+ `set_table_element`（level=table/row/cell 通用元素手术，TBLPR 17 / TRPR 12 / TCPR 13 封闭白名单兼 schema 位插入序；gridSpan/hMerge/vMerge 受保护指路 merge_cells）+ `set_cell_format`（格级段落+字符格式，段落→格内全部段、字符→格内全部 run）+ `merge_cells`/`split_cell`（Word 原生语义：纵并 restart/continue 内容留原格、横并 gridSpan 求和内容拼首格；**网格列区间对齐判据**（同格号 ≠ 同网格列）；结构重构独占一批）。⚠️ 排障纪要：内层元素寻位（find_element_span 返回**切片内偏移**）勿拿去索引原串——须用切片变量索引（凑巧对齐时假绿，tcW 后跟 vMerge 即读垃圾）
- **S3 五波·样式档案与模板个性化（D12，2026-08-25）**：def_edit.rs 定义部件三操作（create_style / set_style_element 容器四档 / set_numbering_element）+ styles/styledef/numbering 三投影 + 顺路件（insert_table_after.table_style / clear_body / merge_cells 矩形区）+ FamilyOp 分族路由（doc/style/numbering 部件互斥）；承载层 word_style_profile 双轨（agent.yaml 块→SystemPromptStage 注入「## Word 文档样式偏好」+ set_agent_word_profile 命令 + 提案通道 ""=摘除 + 前端薄接线）；templates/ 目录=约定不创建
- **S3 六波·生产反馈修正三件（D13，2026-08-26）**：① 缩进跨层压制（**apply_para_formats + fresh_ppr_inner 双位点**——chars 单位变体显式写 0 才压得住样式层 firstLineChars 透出；删同层变体=样式层透出假成功；零值连 hanging 系四零、非零只压 chars（元素内 hanging 优先 firstLine，写 0 反伤））② set_cell_format 加 style（格内全段 pStyle 手术，名/ID 反查同 set_style；validate_formats 带 style_present 纯样式操作合法）③ ppr 投影 row+cell 下钻格内逐段 pPr（守卫 drills={tblpr,ppr}，row 需搭配 cell）。方法论：agent 缺口报告先对码核实再定级（「缺功能」可能实为跨层 bug）
- **S3 七波·删行+批组合三放宽（D14，2026-08-26，66c55cf 随 0.5.4 发版）**：① `delete_table_row`（P0 真缺位——agent 曾被逼用 trPr hidden 藏行）：结构重构独占一批；纵向合并三态守卫（合并头+下方续格→拒指路 split_cell / 纯续格行可删链条缩短仍合法 / 普通行直删）；仅剩 1 行拒指路 delete_block ② 同格多元素同批：used_cells 去重键加目标键——set_table_element 按元素去重（vAlign+tcBorders 一批序无关），内容/格式手术保持每格限一条（重写语义）③ 同锚多段链式：同锚块多条 insert_paragraph_after 按输入序排列——apply 聚合进**单个插入 splice**（镜像 table_plan_idx；勿做偏移数学，块间缝隙可为 0 会伸进相邻 splice）④ description 澄清两处非缺口：跨表格块本就可同批（agent 因描述不清拆 5 批）+ 列宽按内容配方。批组合放宽共性判据：**序无关/序确定才可组合，重写语义保持互斥**
- **八波·验收/范围锁/值优先 + 委派两件（D15，2026-08-31，随 0.6.1）**：㉒ `validate_docx` 纯读断言工具（6 kind：block_count/table_shape/block_text/block_style/cell_text/cell_paragraph_count；上限 50 全评估不短路；**断言失败=正常输出 passed=false 非 Err**——doom_detect 按 is_error 计签名，验收结果是数据；越界地址=该条 fail 附实际范围）㉓ edit_docx `allowed_blocks` 范围锁（顶层 [lo,hi]；ClearBody+锁拒、style/numbering 族+锁拒；拒批全有或无逐字节 untouched）㉔ insert_table_after `rows_text`（Markdown/TSV 与 rows 互斥，等价输入产物逐字节相等；格内单段诚实边界）㉕ 委派进度报告（collect_progress 从子会话 session_events 提取，**恒进 result JSON 正常完成也带**）㉖ doom nudge `==`→`>=`（streak 3/4/5 各一次）
- **九波·write_docx 模板优先生成（D16，2026-08-31，随 0.6.1）**：`blocks`（heading/paragraph/table）一次调用从模板生成完整文档，取代 copy_file→clear_body→逐块写 N 轮；编排 = clear_body→注入裸锚→正序批次（连续段聚批、表独批）→删锚；段落恒带显式样式（heading 六候选链）；生成自检=validate 引擎逐块断言**不过不落盘**；错误家族四前缀 `模板无效:`/`模板不支持:`/`生成块无效:`/`生成自检失败:`；纯创建降级延后（§八）
- **D17 共享模板目录（2026-08-31，随 0.6.1）**：`formal-report.docx` `include_bytes!` 资产随包分发（doc/assets/），boot ensure 落盘 `%APPDATA%\com.icepaw.app\templates\`（**存在不动/删则重建**——用户改样式永不覆盖）；解析四层链 workspace templates/ → 共享目录 → 内置档位兜底（**无扩展名相对名补试 .docx 变体**否则文件覆盖档位永不成立）→ 绝对路径；D7 净化豁免仅此一份资产（业务词表终验 0 命中）；⚠️ 模板手术教训：**CT/rels 永不重写只裁剪（重编号=Word 拒载）**
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

### 视觉能力统一适配（事2 / 方案 C；**两档制重构 2026-08-27**）
4 个 Image 块注入入口统一走"按有效视觉能力适配"，杜绝向非视觉模型塞 Image（→400/"看不到"）：
- **能力探测**：`provider/model_info.rs::effective_supports_vision(agent.supports_vision, provider, model)`——OR 关系（agent 显式 =1 权威；=0 按模型表自动探测，如 MiniMax-M3）。零 schema 改动。
- **两档制路由（2026-08-27 重构，取代旧三环兜底）**：① agent 有效视觉 → Image 直过（链不被咨询）；② 非视觉 agent → **平台视觉配置链**（「设置-通用-视觉读取」条目数组：主模型 + 可选降级，按序尝试首个成功即用）；未配置 → 剥离 + 诚实提示指路设置页。旧环②（agent 借自身厂商凭据）环③（GLM 视觉 MCP env）已删——环③ 系假环：@z_ai/mcp-server 内置 GLM-4.6V 不可控且 Coding 套餐专属，借到的 Coding key 打标准端点必 1113；glmMcpTemplates 视觉模板同步撤下。
- **存储**：pref key `vision_config` = `Vec<VisionConfigEntry{provider,model,api_key,base_url?}>`；**Some=权威（含 Some(vec![]) 显式清空）、None=回落旧四键单条目**（读侧兼容零迁移）；旧三环键写入侧已死。视觉端点表在 `harness/vision.rs` 独立于 chat 注册表（minimax chat=Anthropic 协议但 vision=OpenAI 兼容端点，非重复代码）；条目 base_url 显式优先、否则按厂商默认——**端点成对原则：key 与端点同一鉴权域，不猜第三方**。
- **统一适配**：`harness/modal.rs`——`gather_vision_candidates(pool)`（= 读 vision_config 条目链，单次 DB 读）/ `adapt_blocks_for_vision`（有效视觉原样过；非视觉逐图代读成 Text、失败剥离+诚实提示）/ `strip_image_blocks_to_marker`（历史静默剥离）。代读 marker 带实际读图模型 `[图片经视觉模型代读为文本（{model}）]`——降级换模型对用户与 agent 可见。
- **4 入口接线**：① 用户上传+③ 历史 → `context/stages.rs::ModalCapabilityStage`（Pipeline，TokenWindow 后 Final 前）；② 工具返图 → `tool_executor` 注入 Image 前查 effective_vision（时序独立于 Stage——工具循环后续轮次的图进不了 Stage，必须在此守卫）；④ `view_attachment_image` 判断改 effective_supports_vision + 凭据收集复用 gather。
- **utility thinking 唯一真相**：`provider/model_info.rs::utility_thinking(provider, model)` 三态（None 非智谱不注入 / Disabled 智谱非 5.3 注入 `{"type":"disabled"}` / LowEffort 智谱 glm-5.3 子串注入 enabled+reasoning_effort=low——5.3 全系拒 disabled 硬发必败）；摘要通道（openai stream_summary）与 describe_image 共用，勿再各自硬编码。
- **健康检查**：`test_vision_config` 命令（设置页逐条「测试」按钮）——1×1 PNG 探针走 `entry_to_credential` + describe_image 全链路（同端点推导/同 JSON 形状/同思考策略），测过=正式可用。
- **⚠️ 不变式**：任何新增的 Image 块注入点都必须经 `effective_supports_vision` / `adapt_blocks_for_vision`，不得对非视觉模型直塞 Image；视觉兜底凭据只来自 vision_config 条目链，勿复活借凭据暗通道；新增 message-kind emitter 的 Image 存储仍走 refify_blocks（见会话事件日志节）。

### 会话事件日志（Phase 0+1+2A 已发布；Phase 2B 退役三件套已落地 2026-08-17）
单一 append-only 事件日志基石（锁定愿景：统一 session / 多 agent 图协作 / 轨迹可还原）。
- **表**：migration 44 `session_events`（seq INSERT 子查询原子 + UNIQUE 兜底；message_id 故意无 FK——事件须活得比被删占位行久）
- **词表 14 kind** + typed emitters：`harness/event_log.rs`（EventCtx + warn-only 影子定位；14th = `context_breakdown` ③ 可观测化——一条事件装整回合的段级组成+指纹+逐轮 usage 序列，emit 单点在 stream_loop wrapper（inner 返回后，恒落 turn_ended 之后），derive skip 臂同 turn_context 构建期元数据）
- **接线全退出路径**：chat_cmd/memory/loop_engine/cleanup(PersistOutcome)/retry_round/tool_executor/stages；**硬规则：事件 inline `.await` 禁 spawn，turn_ended 必须先于 cleanup() unregister 落库**
- **supersede**：自动续写同 message_id 多条 assistant_message，回放 last-wins
- **导出**：`export_session_trajectory` 命令 → JSONL（docs/backend-api-reference.md）
- **Phase 1 对账（889e9a8..0baa06c，6 commits）**：`harness/derive.rs` 纯回放（supersede last-wins / 空回退对称 / 坏 payload 记 issue 不吞）+ `harness/reconcile.rs`（A 侧 legacy 行提取走 `list_all_by_rowid` rowid 全量序 + 同一 `parse_content_blocks`；B 侧事件回放；turn 锚点走查分组）。diff 五类 MISSING_IN_DERIVED/MISSING_IN_LEGACY/CONTENT_MISMATCH/ORDER_MISMATCH/DERIVE_ISSUE = bug 清单；skipped 全部已文档化容忍（pre_phase0/epoch/incomplete_turn/error_row/discarded_row/empty_placeholder）。`reconcile_session` 命令只读出口。**真机验证：9a2a1968（20 事件 9 行）diffs=[] 且 skipped=[]；eae6d983（36 行零事件）全落 pre_phase0_no_events**。⚠️ 不变式：turn_id == user_msg_id；对账平面 = 行级原始形态（不跑 sanitize/投影）
- **Phase 2A 读路径切换（已随 0.3.6 发布）**：事件日志从影子升格为**干净会话的主读路径**。`harness/read_route.rs` 按会话路由：有事件 + 对账零 diff + 纯事件纪元 → **Derive**（`load_history_from_events` 派生 `Vec<MessageRow>`，锚回真 rowid，走与 legacy 完全相同的下游 Pipeline；派生输出与 legacy 同构同函数，reconcile 已证逐字节相等；摘要 `source_rowid` 取真 rowid 保连续性）。**指纹缓存** `(max_seq, max_rowid)` 追踪新数据（每轮刷新）；原地篡改不被察觉但活跃会话下轮即刷新、休眠会话不被读——`reconcile_session` 命令始终新鲜。诊断 `get_read_route_status` 命令。**不变式**：派生 MessageRow 必须能过 `load_history_with_window` 产出与 legacy 完全相同的视图（含 source_rowid）。
- **Phase 2B legacy 拼装退役（S1 阶段 1，2026-08-17）**：`load_history_from_events` 成为**唯一**生产读路径（session_runner 恒走派生）；`resolve()` 降级为健康监控——非绿（no_events / reconcile_diffs / mixed_epoch）时 error 日志后**照常派生**（历史可能缺行，不再静默回退 legacy；写路径 bug 不再被自动兜底静默吞掉）。排查走 `reconcile_session` / `get_read_route_status`；偏好回滚开关随分支删除（`session_read_path` 不在 KNOWN_KEYS，库内残留无害）。**回滚 = revert 阶段 1 commit**（messages 表双写持续，Legacy 拼装可整体恢复，零数据损失）。零事件会话行为锁定（e2e 场景 7）：派生空历史 + 回合照常完成 + boot backfill 自愈。
- **Phase 2B 前置 backfill（已落地 2026-08-17，3 commits，未 push）**：`harness/backfill.rs` boot 幂等扫尾——给零事件旧会话反向合成事件（reconcile 的逆函数：同 `parse_content_blocks`/空回退对称/同容忍清单 → 构造性零 diff → read_route 现有判据自动放行 Derive，read_route/derive/reconcile **零改动**）。范围只补零事件会话（混合纪元不补——seq 追加语义装不进历史前缀）；`turn_context` 不合成（payload 要 provider/model 快照，旧行没有，填当前配置=伪造）。actor=`backfill` 行是派生数据非运行时事实（append-only 边界的显式例外，重跑可删）；termination=`backfill` 诚实标注；created_at 直传行时间戳。**版本化重跑**：BACKFILL_VERSION 存 preferences，代码>库内 → 纯 backfill 会话删旧重写自愈；**冻结规则**：混入真实事件后永不可重写（会错序），frozen 仅计数。绕过 append_event 走 repo 批量（显式 seq/单会话事务/不广播）。7 测试：全形态零 diff+Derive green / 视图等价 / 幂等 / 混合纪元不碰 / 孤儿行降级 Legacy / 版本重跑 / 冻结。待真机验收：boot 日志 `[ice_paw.backfill]` 行 + 旧会话 `get_read_route_status` 变 Derive。
- **Phase 2B 阶段 2 摘要锚点 seq 化（2026-08-17）**：migration 46 `covered_until_seq`（= 被覆盖消息首现事件 seq，与 derive 排序位严格一致）+ 存量回填；`SummaryState`/insert/update/SELECT 双写双读；`ChatMessage.source_seq`（`#[serde(skip)]`，不进 LLM payload）；锚点定位 seq 优先 `.or_else` rowid 兜底；`SummaryPayload.covered_until_seq`（`#[serde(default)]`，旧事件零迁移）。显式双写过渡，回滚干净（列闲置无害）。
- **Phase 2B 阶段 3 Image 双份存储治理（2026-08-17，3a 读侧 + 3b 写侧）**：消息类 payload 的 blocks 用 `PayloadBlock` untagged 双形态——`Full(ContentBlock)`（v1 内联，旧事件零迁移可读）/ `ImageRef{message_id, block_index}`（v2，字节只在 messages 行）。写侧唯一入口 `refify_blocks`（emitter 字段式签名内部做，调用方传与落库同值的 blocks）；读侧三路水合：derive `hydrate_image_refs`（纯同步 resolver 注入；未命中/越界/非 Image 降级 `Text("[图片内容已不可恢复]")`）+ `to_content_blocks` 防泄漏最后闸 + conversation_cmd JSON 级水合（list_session_events/export，前端零改动）。BACKFILL_VERSION=2（纯 backfill 会话删旧重写自愈，冻结会话保留 v1 照读）。**⚠️ 不变式：session_events 消息类 payload 禁止内联 Image base64——新增 message-kind emitter 必须经 `refify_blocks`，读侧必须经 `hydrate_image_refs` 水合后才能进对账/LLM 视图（ref 形态不得以非 Text 形态流出）**。

### 跨会话通讯（MA-3，2026-09-09 落地 0.7 批 B；2026-09-10 测试反馈修复批 + 二轮实测修复批）
与委派互补的异步对等通道：委派 = 同步阻塞、父 token 级联、深度=1、任务单元；MA-3 = 异步不阻塞、无级联、对话单元。会话 A 的 agent 调 `send_message_to_session` 向会话 B 投递，B 排队、空闲（或经批准）时消费一回合。
- **投递语义（零冲突基石）**：pending 来件只 append `session_events`（`cross_session_message` kind，**不写 messages 表**——与目标在途回合零冲突，单写者仲裁由 chat_state.start 自然保持）；**消费 = 目标会话跑一回合**，复用 `run_agent_turn` 全链路（预算/工具/hooks/事件照常），物化一条带来源标注的 user 消息——**derive 零改动**（user_message kind 照常派生，payload 增量字段 serde default 零迁移）。终态 `cross_session_message_settled`（action=consumed|refused，by=auto|user-approval|user-refused）；**pending 定义 = 有投递无同 message_id 的 settled**（find_open_turns 同款 NOT EXISTS SQL）。
- **收件三态**：`conversations.inbox_policy`（migration 52，2026-09-10 起默认 **accept**——「在 A 发起、还要切到 B 去批准」反人类）——accept 自动消费（默认）/ hold 扣住待用户批准 / refuse 投递方工具立即 Err（refuse 在政策闸最前无条件拦——用户治理权最大）。**is_reply 例外**：expect_reply 的回投在 hold 政策下也免扣自动消费（对话要闭环，但用户已显式 refuse 仍拦）。粒度=会话；只投 kind='chat' 会话（delegation 子会话拒——防侧信道绕过深度护栏，工具注册条件与 delegate 同源）。
- **项目边界（硬边界，2026-09-10 拍板）**：投递仅限**同项目**会话之间——`project_boundary_error` 纯函数（inbox.rs）：源与目标都挂项目且同 id 才放行；跨项目/单散落/双散落全拒（三段式文案指路「挂同一项目或改投同项目其他会话」）。校验在**执行期 deliver 内**（非注册期隐藏——跨项目看得到列表但投递报错，可见性 ≠ 写权限）；回投 SourceInfo 取 conv(B).project_id 与 A 同项目（原投递已校验），边界自然通过对称。
- **投递工具两件**（壳 `harness/mcp/relay.rs`，引擎 `harness/inbox.rs`）：① `send_message_to_session`——`target`（id 或标题唯一匹配，重名不猜）/ `content`（≤8000 字符超限拒收不截断）/ `expect_reply`（默认 false）。授权 `Always` 级——授权决策点是收件三态不是弹卡；held/queued 是入队不是工具同步等待（立即返回状态 JSON，批准动作在收件箱）。② `list_conversations` **寻址发现**（2026-09-10）——全量会话概述元信息（id/标题/所属 agent/所属项目/更新时间，**零内容**；kind='chat' 过滤、更新倒序、OVERVIEW_CAP=100 截断+诚实 note；agent 删除回退「（agent 已删除）」）。治「投递必须点对点但 agent 查不到地址、用户拿 @会话 兼职报地址」的语义混乱——@ 恒为「向内引用」（快照注入）不兼职寻址。两工具同 conv.kind=='chat' 条件注册 + PLATFORM_TOOLS 白名单（enabled_tools 收窄不断跨会话通讯）。
- **消费引擎**（`harness/inbox.rs`）：触发源——投递时（accept 或 is_reply×hold）且空闲即 spawn / accept 会话回合结束（turn_ended 广播 → drain watcher 等静默 2s×15s 上限 → 链式排空）/ hold 会话用户收件箱批准。**settled(consumed) 在 chat_state.start 成功后、spawn 前 append**（原子占位防双击双消费）。回合成败都算已处理（失败在会话内有 message_error 事实）。
- **来件双块结构 + 元数据（2026-09-10 结构化）**：`compose_incoming_blocks` = [Text(标注), Text(正文)]（LLM 两块自然可读）；`compose_incoming_annotation` 独立标注块，**回信指引按 expect_reply 分叉（2026-09-10 二轮·双投递根治）**——false → `…｜如需回复用 send_message_to_session 工具，target={conv_id}]`；true → `…｜对方已开启自动回传，直接作答即可（无需调用投递工具）；如需另行主动投递，target={conv_id}]`（指引直接作答防「手动回一条+自动回投一条」双投递；**两分支都保留 `target=` 锚**——前端文本兜底解析依赖，形状锁 BACKEND_SAMPLE/BACKEND_SAMPLE_REPLY）；`compose_incoming_text` 扁平快照（messages.content 检索面）。**incoming_source 元数据三写**（前端 incoming 卡的权威数据源）：messages 列（migration 53；`set_incoming_source` 二段写——update_content_blocks 同款，NewMessage 不扩字段防全量构造点改动）+ user_message 事件 payload（IncomingSourceMeta 三字段，serde default）+ derive 透传（DerivedMessage → to_message_row 序列化回 JSON）。前端解析两级：`incomingInfoOf` 元数据优先（格式演进免疫；正文取 content_blocks 末 Text 块，坏 JSON 回退扁平剥头）→ `parseIncomingText` 文本解析兜底（legacy 消息；**格式改动两边同步**，形状锁 crossSession.test.ts 的 BACKEND_SAMPLE）。
- **expect_reply 回投**：消费回合完成 → 目标 agent 最终回复投回源会话（同一 deliver 复用，政策照查对称）；回投带 `is_reply=true`（事件 payload 字段 + InboxItem 投影 → 前端「回复」pill）；回投恒 expect_reply=false **链一次止**；回复为空不投。**双投递防重（2026-09-10 二轮）**：`MANUAL_REPLY_GUARD`（键 `(源会话,目标会话)`→登记时刻）——deliver 的 is_reply=false 路径登记（自动回投自身不登记防自吸收），consume_pending 在回投 spawn 内 `done_rx.await` 之后 `manual_reply_take`（消费语义；`t >= turn_start` 只认本回合内），命中则跳过自动回投（agent 已手动投回，再投即重复——不依赖模型听话的兜底闸）。
- **护栏**（常量在模块顶）：pending 队列上限 10（超限 Err 拒收）/ 同会话 10 分钟窗口自动消费 ≤6 次（内存窗口重启清零；超限留队待手动放行——**用户批准不占配额**）。
- **命令四件**（`commands/inbox_cmd.rs`）：list_inbox（policy+pending 列表含 is_reply，单条坏 payload warn 跳过不挡列表）/ list_inbox_counts（侧栏 badge boot 批量）/ set_inbox_policy / respond_inbox_item（批准时会话忙 → Ok(false) 转 Err 三段式文案，来件留队零丢失；**busy 文案按语境分叉**——auto 语境[`policy==accept || is_reply`]「正在处理上一条来件，本条已在队列中会自动处理」，hold 保持「请等本轮结束后再批准」）。
- **前端五件**：`useInbox`（`session:event-appended` 总线过滤两 kind 增量维护计数 Map + 失焦 OS 通知按政策分流文案[hold=待批准/accept=知会收到]恰一次 + refreshInboxCount 权威回正）/ ChatHeader 收件箱 popover（InboxPopover：**条目双视图**——auto 语境[accept || is_reply] 队列视图[「排队中 · 会话空闲后自动处理」标注 + 次要「立即处理」钮（不摘——配额尽时手动放行是唯一通道，title 说明）+ 头部「N 条排队中」]、hold 审批视图[「批准并消费」主色]；拒绝两步确认武装态 + 政策 segmented 乐观切[accept 标「默认」] + is_reply「回复」pill；**ChatHeader z-index=dropdown(100)**——popover 困在 header 层叠上下文内，badge(10) 低于 tabbar(raised 20) 会被盖，父层提高子层跟父）/ Sidebar 会话行 badge / incoming 卡（ChatMessages，元数据优先）/ 轨迹 CROSS 行（useTrajectory 词表）。
- **⚠️ 不变式**：① 计数是气味不是真相——settled 本地算术抵扣可漂移（bus 丢帧），popover 打开与处置后必须走 list_inbox 权威刷新；② turn_id 归组键三侧同一 `cross:{message_id}`（投递/消费/拒绝），actor=`agent:<源id>` 诚实归因（中继消息不带用户权威）；③ derive skip 臂必须含两新 kind（否则 DeriveIssue 污染对账——顺手收编了 model_switch 既有漏项）；④ OS 通知不传 request_id（toast 按钮是工具授权 oneshot 协议，与收件箱处置不同域）；⑤ 项目边界是执行期校验非注册期隐藏——list 全量可见 + send 层拦截是刻意设计（能列目录 ≠ 能写），勿把边界下沉到发现层；⑥ incoming_source 元数据与标注文本双轨并存——元数据为前端权威，文本头为 LLM 可读事实与 legacy 兜底，格式改动后端组装/前端解析/形状锁三处同步。

## 当前状态（2026-09-10）
- **MA-3 二轮实测修复批已落地（2026-09-10，未 commit）**：用户真机 A↔B 实测两反馈（数据取证见 session_events：B 每轮回 2 条）。① **双投递根治两道闸**——标注按 expect_reply 分叉（true → 指引「直接作答（无需调用投递工具）」，false 保持投递工具指引；两分支保 `target=` 锚）+ `MANUAL_REPLY_GUARD` 防重兜底（消费回合内已手动投回源会话则跳过自动回投，不依赖模型听话；回投 spawn 内 done_rx.await 后检查，登记/检查均内存态）；② **收件箱队列视图**——accept/is_reply 条目「排队中 · 会话空闲后自动处理」标注 + 批准钮降级次要「立即处理」（不摘——配额尽时手动放行是唯一通道），hold 保持真审批「批准并消费」；③ **busy 文案语境分叉**（inbox_cmd auto 语境「已在队列中会自动处理」vs hold「等本轮结束后再批准」）；④ relay.rs 工具 description 补「expect_reply=true 时目标会被指引直接作答，勿自调投递工具回复」。顺手：头部计数 accept 下「N 条排队中」。cargo 1473 / vitest 510
- **MA-3 测试反馈修复批已落地（2026-09-10，已 commit bfebf6b/41816d9/e70d302 未 push）**：用户实测四反馈 + 补充一件。① 默认政策 hold→**accept**（migration 52 DEFAULT 重写——dev 库实测 52 从未成功应用 max=51 改值安全；「在 A 发起、切到 B 批准」反人类）+ **项目边界硬校验**（`project_boundary_error` 纯函数：同项目才放行，跨项目/散落全拒三段式指路；执行期 deliver 内拦非注册期隐藏）；② **expect_reply 回投免 hold 扣**（deliver 增 is_reply 参，accept ∨ (is_reply×hold) 自动消费，refuse 恒拦；回投带 is_reply 元数据全链路至 InboxItem「回复」pill）；③ **popover 层级修复**（ChatHeader z badge→dropdown(100)——popover 困在 header 层叠上下文，badge(10) 输给 tabbar(raised 20)，父层提高子层跟父）；④ **incoming 双块结构化**（[Text 标注, Text 正文] + **incoming_source 元数据三写**：messages 列 migration 53 / user_message 事件 payload serde default / derive 透传；前端 `incomingInfoOf` 元数据优先→parseIncomingText 文本兜底）；⑤ **list_conversations 发现工具**（全量会话概述零内容：id/标题/agent/项目/更新时间，kind='chat' 过滤+倒序+CAP 100 截断；治「投递点对点但 agent 查不到地址、@会话 兼职寻址」语义混乱——可见性 ≠ 写权限，边界拦在投递层）+ toolLabels 两词目。顺手件：inbox_cmd InboxItem.is_reply、useInbox 通知政策分流（accept=知会/hold=待批准）、伪造 49 库测试补 messages/conversations 最小表。cargo 1471 / vitest 506
- **0.7 批③ 上下文开销可观测化已落地（2026-09-10，98e4c8d 后端 + 9cf769f 前端 + 文档 commit；未发版未 push）**：BudgetPill miss 归因 chip + 轨迹页「上下文组成」体检区。后端五件套：SystemPromptParts 五段化（joined() 字节等价测试锁）/ `context/anatomy.rs` 事后纯函数聚合（Pipeline 终态 = Model-visible 口径；FNV-1a 12hex 跨版本稳定）/ os_context `OS_HASH_EPOCH` 稳定核（秒级时间行=恒真噪声，冻结后哈希只随真实环境变）/ session_events 第 14 kind `context_breakdown`（一条事件装整回合：segments+指纹三元组+rounds 逐轮序列；emit 单点在 stream_loop wrapper，恒落 turn_ended 后）/ `loop/turn_cost.rs` attribute_miss 判定表（7 slug；门槛 prompt≥1024 且 cached==0；跨回合基线 last_breakdown_payload last-wins）。fingerprint.tools 取轮 0（回合起点基线——末轮是相关性裁剪子集，比对恒真噪声）。前端五件：types/missHint（slug 中文词表两边镜像；机理说明只住前端——后端只判事实）/BudgetPill miss chip（仅 first_request→null 弱展示）/useTrajectory 折头/TrajectoryInspector 组成区（主色单色条形+偏差行+逐轮徽+披露脚注）。⚠️ 不变式：归因措辞诚实（本地推断非 provider 事实）；词表改动三处同步（event_log 词表 + derive skip 臂 / EV_KIND_TO_FILTER 与 summarizeEvent / miss_slug 与 missHint）。cargo 1465 / vitest 501
- **0.7 批 B MA-3 跨会话通讯已落地（2026-09-09，cacaa70 后端地基 + b9bea59 通道引擎 + 083f32a 前端接线 + 文档 commit；未发版未 push）**：`send_message_to_session` 异步对等通道——pending 只入事件队列（不写 messages，与在途回合零冲突）、消费复用 run_agent_turn 全链路物化带来源前缀的 user 消息；收件三态 hold 默认（migration 52）、触发三源、expect_reply 回投链一次止、护栏常量四件；前端 useInbox/收件箱 popover/侧栏 badge/incoming 卡/轨迹 CROSS 行。详见「跨会话通讯（MA-3）」节。cargo 1443 / vitest 484
- **0.7 批 A 首件已落地 db29be7（2026-09-09，发版后 commit push；手测随 2026-09-09 拍板整体通过）**：委派子会话降级链继承——`delegation_fallback_plan`（子 agent 自链优先，无链继承父 agent 配置链并摘除子主档同 id 档；父行读失败降级无链不阻塞委派）。核查发现子 agent 自链 0.6.15 已接线（delegate 与主链共用 `production_fallback_plan`），真缺口仅「无链继承」。cargo 1433
- 版本 **0.6.16 已发版 push**（= 0.6.15 + **健康检查批次 R**——五路扫描 30 条入台账的四批修复：行为五件 ae31144[R1 提案卡绕引用语义守卫 + R2/R3 chat store 竞态 + R4/R5 keep-alive 成对] + 前端四件 3693b7f + 视觉三件 07ec5d6 + 文档八件 0ad03d0，明细见下条；**0.6 线收官**，0.7 蓝图四件已拍板见 memory 0-7-dev-plan：降级链继承 / MA-3 跨会话通讯 / 上下文开销可观测化 / diff 侧栏；cargo 1428 / vitest 464）
- **健康检查批次 R（2026-09-09，发版后体检；四组已 commit，dev 环境手测通过）**：五路扫描（后端不变式/ModelProfile 线/错误路径并发/前端/文档对账）30 条入 docs/tech-debt-ledger.md 批次 R——干净面：后端十条不变式 9 条成立、错误路径 P0/P1 零发现、fmt 零语义损伤。四批修复全推进：**行为五件**[R1 提案卡绕引用语义守卫（agent_cmd.rs validate_update_model_fields_conflict 扩分支：行已引用态 + model_profile_id 缺席 + 快照族手填 → 拒）+ R2/R3 chat store 竞态（await 后 activeConvId 守卫 + sendMessage 闭包捕获 roundConvId 防 sendingConvId 易主 + catch 清 bgStreams）+ R4 LogSettings/R5 useProjectTasks keep-alive 成对（listenerLive gate 样板）] + **健壮性五件**[R6 解析失败按步骤分流 ResolveProfileError{NotFound,Corrupted}——仅行缺降级 legacy、槽位坏上抛（⚠️crypto 对槽位无记录也返 NotFound，按错误变体分会误归）+ R7 set_mcp_enabled 吞错上抛 + R8 checksum 自愈真实计数 + R9 Mock 挂 #[cfg(test)] + R10 KB 初始索引失败 warn] + **视觉三件**[R11 暗色错误横幅/附件卡 token 化 + R12 z-index/字号/间距零头收编（⚠️ChatHeader z:1 选 badge 非 base——.chat-render 双 pane 是 absolute 压过非定位）+ R13 死样式删] + **文档八件**[版本号三处统一 0.6.15 / CHANGELOG 补 0.6.4+0.6.15 两档 / CLAUDE.md 状态节+ModelProfile 专节+架构树 / CONTRIBUTING·architecture·roadmap 勘误]；R14-R19 观察池。cargo 1428 / vitest 464
- 版本 **0.6.15 已发版 push**（tag v0.6.15 + GitHub Release 带 NSIS exe；= 0.6.10 + 中间测试包 0.6.11~0.6.14[仅装机未发布，版本号未使用] + **模型配置实体化（ModelProfile）全线**：Phase 1 b2f079b[实体库+Stronghold 槽位+视觉/语义检索引用——Key 一处换全局生效；通用页管引用/模型页折叠卡实体库布局，用户拍板] + 通用页显式保存 79c5f53 + Phase 2 三笔 81c35df/cb0a145/54b11a7[agent 挂 model_profile_id 引用四列快照制 + 降级链 fallback_profile_ids 三拦截点换档 + boot 存量抽离幂等标记] + Phase 3 全批 455534c[新建保留手写表单保存时自动物化（去重键复用）/编辑合并 tag 链选择器（vuedraggable+键盘换位）/链路仿真测试 test_agent_model_chain 真发 16 token/健康归因双端根治 demote_unchanged_inputs] + **工具行三件** 1b90915[会话头 agent 名点转深链 + 行级 diff 三计数徽记（edit_file 存量回填前端镜像 lineDiffStat）+ 思考耗时小写] + **白屏二轮根治** bac93c4[WebView2 缓存瘦身：版本升级清 Cache/Code Cache + --disk-cache-size 50MB 封顶（⚠️prune 必须在 WebView2 启动前、additionalBrowserArgs 整体替换须带默认三条）+ 主窗 visible:false 双 show（前端 rAF + 后端 10s 兜底）+ boot 锚点日志] + **全仓 cargo fmt 规整** 6c8b716[84 文件零语义变化——工作区曾经历非标准紧凑化格式] + 设置页内容列限宽 1ba152e；cargo 1427 / vitest 460）
- 上一版 **0.6.10 已发版 push**（= 0.6.4 + 中间两轮测试包 0.6.8/0.6.9[仅装机未发布，0.6.5-0.6.7 版本号跳过未用] + 发版批：**读路径热路径三减** 12fd66a[发送每轮 O(N)——reconcile 后台化 + derive supersede 哈希 + tail-limit 前置，千轮 bench 1562→350ms；⚠️非绿检测延迟最多 1 turn、scopeguard 建在 async 块内] + **0.6.8 批五件**[工具行级摘要 150fefa（单行形态+思考耗时持久显示+委派卡精确绑回）/ 启动白屏根治 d2240fe（孤儿扫尾误匹配死循环+启动重活后台化+index.html 首帧骨架）/ run_command 引号根治 ee08f4a（raw_arg 直通道+kill_on_drop+回显防御）/ 探测链三小件 3c59caa（超时 10/20s+网络错误文案+base_url 双层 Option）/ 工具层二轮 7d14566（7 文件错误契约补课+设置页工具集搜索分组中文化）] + **0.6.9 批**[运行态 glyph 双形态定稿 b440570——工具调用行 spinner、其余九宫格（68a3799 回归修复后收口）] + **发版日三件**[工具展示名扩全 40 条中文 4643901 / 轨迹表思考续写图标双行根治 84aa6c5（⚠️lucide 进文本流一律显式 inline-block——base.css 全局 svg display:block reset 陷阱）/ 轨迹页事件类型筛选两件 21a3888（「仅对话」预设+类型多选下拉操纵同一 hiddenKinds、持久化 icepaw-traj-hidden-kinds 切会话不重置、「加载更早」scrollTop≤80 淡入门控）] + **发版日生产实案修复** 02b3edc[已删项目页死态常驻——keep-alive 缓存复活兜底（onActivated/watch 三路）+ 启动恢复死路由拦截（planRestore 加 allProjectIds 第 4 参，归档仍可直链）；⚠️外层 keep-alive 无 :key 是刻意的（会话切换不重挂载语义），布局级死态必须自兜 onActivated]；cargo 1350 / vitest 416）
- 上一版 **0.6.4 已发版 push**（= 0.6.3 + **2026-09-04 质检批次 Q 四组搭车**：核心批 7e374e6[copy/move 双路径越界授权改 all-match + doom 错误签名剥变体前缀壳 + 外部 MCP 超时清 pending 表 + event_bus Lagged 自愈] + 视觉规范批 40b164e[语义色圆点替 🟢🟡🔴/轨迹·KB·审批卡 Lucide 收编/tokens.css 自动暗区镜像补齐/间距令牌化/TrajectoryView keep-alive 监听成对] + 性能批 8f7d0b7[attachments·indexer·docx 同步重活 spawn_blocking 三件 + KB 语义检索四标量签名向量缓存——⚠️签名顺序不变式：签名先于数据读；rowid 回收陷阱靠 SUM(LENGTH(content)) 第四标量兜住] + **委派审批改造** d6c8ff0[L0 授权记忆会话级 AuthSessionRegistry + L2 委托时预授权两档，见 memory] + **审批通知直操作** 7838add[toast 批准/拒绝按钮 + 点主体前置主窗 + single-instance 防双开] + 通知恰一次 e906bf8 + 多轮工具回合冻结轮渲染修复 c9d2680[骨架门控改 item 级 isLiveAssistant] + CI Linux cfg 分支修复 dcf7884；cargo 1332 / vitest 362）
- 上一版 **0.6.3 已发版 push**（= 0.6.2 + **侧边栏收起 rail 模式**[56px 单列行动栏 + 会话/项目 flyout + 主题钮迁 footer 恒 ≤1 实例 + ProjectSwitcher collapsed 变体菜单头部两入口，c4eb90d] + **底缘渐隐置底联动**[对话/轨迹两 tab 置底不透明，6cccabf] + CLAUDE.md 图标包名勘误[@lucide/vue]；vitest 341）
- 上一版 **0.6.2 已发版 push**（= 0.6.1 + **enabled_tools 旧白名单复活根治**[0.6.1 生产实案修复：镜像同步 DB 列 + 组装收窄披露日志，80bba3d] + **Word 十波 D18 TOC+图片插入**[write_docx toc/image 块 + edit_docx insert_toc_after/insert_image_after + validate block_image/block_field + inspect 段尾标记 + 包级只增补通道] + **十一波 D19 生产坑三件抽象化**[占位段不是内容/重写按位继承/表内地址模拟器] + **生成期切项目详情卡顿三修**[bgStreams 原地 mutate/轨迹监听 keep-alive 生命周期/尾页 SQL 先 id 后回表，4ad70ef] + 输入区底缘渐隐；cargo 1317 / vitest 330）
- 上一版 **0.6.1 已打包未 tag**（= 0.6.0 + Word 八波验收五件 + 九波 write_docx 模板优先生成 + D17 共享模板目录；生产机已装机——enabled_tools 实案即出自此机，修复随 0.6.2）
- 上一版 **0.6.0 已打包已 push**（tag v0.6.0 + GitHub Release 带 NSIS exe——首次正规 tag/Release 流；= 0.5.5 + **Computer Use 全线**[看屏三件+操作七件+act-and-look+屏幕共享通道治理五步，见「屏幕读写」节] + **视觉读取两档制重构** + **Agent 配置一致性两批** + 新模型跟进[GLM-5.3 系/DS 视觉] + 首用引导批① + 通用设置卡片分组重设计；cargo 1232 / vitest 328；main 与 origin 推平）
- 上一版 **0.5.5**（= 0.5.4 + **换厂商配置分裂根治 + 智谱 Coding 端点显式切换**，两条专段见下；cargo 1138 / vitest 328）
- 再上 **0.5.4**（= 0.5.3 + **S3 七波·删行+批组合三放宽（D14）**：① delete_table_row[P0 真缺位——vMerge 三态守卫：行内合并头下方同网格列有续格拒指路 split_cell / 纯续格行可删 / 仅剩 1 行空表保护指路 delete_block；结构重构独占一批] ② 同格多元素同批[used_cells 去重键加元素名——vAlign+tcBorders 一批序无关组合；组合判据：序无关/序确定才可组合，重写语义保持互斥] ③ 同锚多段链式[镜像 table_plan_idx 的 insert_plan_idx 聚合单 splice，勿做偏移数学——块间 gap 可为 0；链序=输入序] ④⑤ description 两处澄清[跨表同批如全文档统一边框一句话 + 列宽按内容加权 tcW 配方]——生产 agent 缺口报告第二弹，五条对码核实分流，两条实为「引擎已支持、描述不清」；cargo 1129）；再上 **0.5.3**（六波·生产反馈修正三件[D13]：缩进跨层压制[chars 变体显式写 0 双位点] + set_cell_format.style 格内样式 + ppr 投影 row+cell 下钻；cargo 1125）；再上 **0.5.2**（四波·表格格式四件 + 五波·样式档案与模板个性化[word_style_profile 双轨承载]；cargo 1122）；再上 **0.5.1**（三波·表格内容四件；cargo 1071）；再上 **0.5.0**（Word 能力演进整线 + Agent 质量拍 Phase 1；cargo 1060 / vitest 326）
- **换厂商配置分裂根治（2026-08-26 生产反馈，a3d9e8e 随 0.5.5）**：agent 换 provider 后 UI 显示新厂商而 agent.yaml 镜像停在出生值、且端点/key 不联动（旧厂商 key 打新端点 → 报对端厂商错误）。三件：① update() 同步 yaml 的 provider/model/base_url 镜像行（agent_yaml.rs sync_agent_yaml_mirror——信息性镜像运行时不读但用户当真相源；文件存在才补丁/写前闸/原子写/best-effort）② 后端端点跟随厂商（default_url_on_provider_switch：换厂商且未显式给 base_url → 重置注册表默认，防 DB 残留旧厂商 URL）③ 前端换厂商必填新 Key（AgentForm validate：requires_key 厂商切换 + 空 key 拦截；免鉴权 ollama/custom 放行）
- **智谱 Coding 端点显式切换 + GLM 1113 指路（2026-08-26 生产实案，9540093 随 0.5.5）**：Coding 套餐 key 打标准端点报 1113「余额不足或无可用资源包」——**套餐有余额仍报**（Coding 额度只在 Coding 端点生效，标准/Coding 两套端点 key 不通用；「测试连接」自动回退救不了——列模型是鉴权层动作，标准端点也放行，假绿固化错误端点）。三件：① AgentForm URL 框下端点胶囊（endpointOptions：带 alt_urls 的可见厂商才渲染，当前仅智谱）——切换只换注册表地址仍只读防抄错、探测显式传所选端点不走多端点回退、存量按 URL 匹配高亮、切厂商归位默认 ② 错误分类细分 `GlmResourcePack`（措辞含「无可用资源包」智谱专属，须先于 429/余额通用分支）：文案三段式指路端点切换非只叫充值；不可重试与余额不足一致 ③ glm 注册表 note 更新。⚠️ 不变式：测试连接=鉴权层动作，列模型通 ≠ 该端点认可对话权益
- 分支：仅 `main`
- 近期递进：0.4.1 → 质量拍 Phase 1 + Word 能力演进整线（S0a→S0b→手术引擎→S3 首波→真机复盘两批→D9 set_ppr_element）→ 0.5.0 发版 → 生产实战反馈表格双缺口 → S3 三波表格四件（D10）→ 0.5.1 打包 → 生产反馈表格格式缺口 → 四波（D11）→ 样式通用抽象+个性化需求 → 五波（D12 双轨承载）→ 0.5.2 打包 → 生产 agent 缺口报告 → 六波修正三件（D13）→ 0.5.3 打包 → 缺口报告第二弹 → 七波删行+批组合（D14）→ 0.5.4 打包 → 换厂商根治+Coding 端点（0.5.5）→ **Computer Use 批次③+④ 全线 + 视觉两档制 + 配置一致性两批 → 0.6.0 发版（tag + GitHub Release）** → 八波验收五件 + 九波 write_docx + D17 共享模板 → 0.6.1 打包 → 十波 TOC+图片 + 十一波抽象化 + enabled_tools 根治 + 卡顿三修 → **0.6.2 发版 push**
- `cargo test --lib` 1473 passed / 0 failed（+ 集成测试：session_runner_e2e 12、session_reconcile_e2e 6+2 ignored、session_event_log_e2e 3、memory_e2e 3、message_repo 7、provider 11）；clippy --tests -D warnings 0 警告；vitest 510（1473/510 = MA-3 反馈批后 1471/506 + 二轮批新增 2+4：后端 inbox 标注分叉 1 + manual_reply_guard 1；前端 crossSession expect_reply 分支形状锁 1 / InboxPopover 队列视图 3）
- 仍待办：**手测积压 2026-09-09 用户拍板整体通过**——全部历史手测观察点（0.5.x~0.6.16 与 0.7 批 A）不再逐项验证，生产使用驱动：出问题用户会报、报了即转修复（勿再把手测清单端出来催，验收信息以用户反馈为准）。真功能待办：proposal Phase 2（MCP 域）、V5 钩子未用未测、Word 后续波（条件批量替换）；UE5/Word 生产观察仍是 0.7 立项输入（P8 升格 / Computer Use 优化方向）
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

