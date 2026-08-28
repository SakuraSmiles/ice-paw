# Computer Use 能力路线

> 真相源：本文档 + CLAUDE.md「关键系统」节。批次③设计存档在会话记忆（computer-use-batch3-design）。
> 状态标记：✅ 已落地 / 🔨 设计定案待开发 / 💤 待议。

## 定位与愿景

有视觉大模型时，agent 像人一样「看着屏幕操作」：截屏看状态 → 决定动作 → 验证结果，闭环执行桌面任务。产品形态 = **平台级屏幕通道**（类似桌面共享的一键开关），而非散在会话里的逐次工具调用。

## 现状：批次③ 看屏+操作引擎（✅ 2026-08-28，未 push 未手测）

引擎件（10 工具，`harness/mcp/screen/{mod,backend,coords,state,input,keyboard}.rs`）：

- **看屏**：capture_screen（monitor/region 裁剪、长边 ≤1600、5MiB 降级重编码）/ capture_window（PrintWindow 免聚焦）/ list_windows
- **操作**：mouse_move / mouse_click / mouse_drag / mouse_scroll / type_text（KEYEVENTF_UNICODE 逐 UTF-16 单元）/ press_key（组合键解析、VK 表本地定义）/ wait（select 取消令牌）
- **坐标契约**：模型坐标 = 本会话最近截图的图片像素空间；ScreenState（conv→CaptureMeta，64 LRU）+ 布局 revalidate + img→phys→abs 端点精确映射
- **错误家族**：screen 捕获失败 / 坐标基准缺失 / 坐标过期 / 输入失败 / 按键无效 / 不支持
- **授权**：全部 Confirm + auth_reason 披露（list_windows/wait 除外）
- **截图历史压缩双钩**：compact_screenshot_history keep_last_k=3（loop_engine 钩 A + ScreenshotHistoryStage 钩 B；只动 LLM 视图，DB/事件日志保完整图）

v1 边界仍然有效：不做窗口置前；GDI 盲区（DRM/HDR/独占全屏、UAC 安全桌面、提权 UIPI）以家族错误诚实暴露。

### 真机冒烟（2026-08-28，自动化部分 ✅）

`screen/real_smoke.rs`（`#[ignore]` 显式运行，不碰用户焦点窗口）+ `backend.rs` 内 GDI 契约测试（无头、随常规套件）：

- **坐标链实测绿**：SendInput 绝对坐标 → 每台显示器中心 → `GetCursorPos` 读回，曼哈顿偏差 **0**，结束还原初始位置。包围盒不变式（显示器并集 == 虚拟桌面）过。
- **三路捕获绿**：整虚拟桌面 / 单显示器 / 前台窗口 PrintWindow，尺寸断言 + 内容非退化（抽样唯一色 ≥8）+ PNG dump 至 `%TEMP%\icepaw-screen-smoke\` 供人工复核。
- **GDI 字节序契约**：`bgra_to_opaque_rgba`（BGRA→RGBA 换位 + alpha 压 255——GDI alpha 文档未定义，RDP/服务会话见过 0，实测本机 DWM 恰好 255 但不当契约赌）由 `SetPixel` 已知色 → 同形状 GetDIBits 的无头测试钉死；红蓝对调/行序翻转是方差类检查抓不住的，只有契约测试能抓。
- **DPI 基准**：生产由 tao 运行时设 PerMonitorV2；测试进程无 tao，`real_smoke` 自设同档（`Win32_UI_HiDpi` feature）——DPI 缩放机器上不自设会拿到虚拟化坐标，冒烟结论失真。
- **本机事实**：单屏 2560×1440 原点 (0,0)、DPI 96 无缩放——**多显示器（负原点/跨屏 VIRTUALDESK）与 DPI 缩放路径仅单测覆盖，未过真机**（coords.rs 单测已含双屏负原点用例，硬件核对待多屏机器）。
- **产品债（冒烟发现）**：Electron 应用的后台离屏渲染窗（如 `Kook_Off_Screen_Rending_Window_Title`）过 list_windows 五条过滤（可见/有标题/非工具窗/非自身/非 cloaked），PrintWindow 得到纯色帧——捕获链路没错但内容无意义。候选缓解：窗口矩形与显示器求交过滤、或 PrintWindow 后内容退化检测降权。批次④ 前不做，记此备查。

---

## 批次④：平台级屏幕通道 ScreenChannel（🔨 设计定案 2026-08-28，用户拍板）

### 4.1 交互模型

「通道」是授权与可见性的单位，不是物理管道——GDI BitBlt 本就是无状态读取，多会话各自截屏天然零冲突。通道提供：**一份授权（上收自逐工具 Confirm）+ 一个可见运行态（HUD）+ 一套写者仲裁（单鼠标资源分配）**。

三个入口，通道为第一公民：

1. **页面开关**（主入口）：聊天头/输入框工具栏的「屏幕共享」开关，用户手动开/关。
2. **agent 建议**：`request_screen_session` 工具（Confirm + auth_reason「请求开启屏幕共享——开启后本会话可直接截屏与操作屏幕」）。批准 → 通道开启（已开则仅附着本会话）→ 事件点亮 HUD 与开关。拒绝 → 正常错误返回，agent 自行措辞回应。复用现有审批卡通道（oneshot registry + 前端事件），scope 档对该工具无意义，前端特判为单键「开启/拒绝」。
3. **会话内直接调用**（现状保留）：通道 Off 时 agent 仍可调 screen 工具，走逐次 Confirm——向后兼容，但产品叙事上不鼓励（描述文案指路开关）。

通道 Off/On 的工具行为：

| 通道 | 看屏/操作工具授权 |
|---|---|
| Off | 现状：逐次 Confirm 卡 |
| On + 开启者会话 | 免 Confirm 直行（授权已上收给通道开关这个动作） |
| On + 其他会话首次使用 | 走一次「加入共享」Confirm（含 delegate 派生子会话——agent 无权自夺，逐个点头）；批准即附着，之后免 Confirm |

**附着语义**（评审 B5 修订）：聊天头开关 / request_screen_session 批准所附着的会话 = 开启者会话，即刻免 Confirm；其他会话（含子会话）首调 screen 工具弹「会话 X 请求加入屏幕共享」确认，批准后附着。附着瞬间主窗 toast + HUD 计数高亮（高可见——静默附着是安全面禁忌）。回合结束不脱离，通道关闭才清空。

### 4.2 状态机（进程级单例，不持久化，重启即 Off）

```
ScreenChannel（Rust 侧，先例：ScreenState::global()）
├─ status: Off | Active
└─ Active:
   ├─ paused: bool                       // 暂停键（全局读写挂起）
   ├─ hud_monitor: usize                 // HUD 工具栏所在显示器（◀▶ 切换，默认主屏）
   ├─ opened_at: Instant                    // HUD 时长显示（无自动过期，见 4.6）
   ├─ attached: Map<conv_id, AttachInfo> // {agent_name, 会话标题, purpose}
   ├─ token: Free | Held(conv_id)        // 写者令牌
   └─ queue: VecDeque<conv_id>           // 等待操作权的会话
human_active: 去抖旗标（最近 2s 有物理输入 → true；优先级高于一切）
```

`purpose`（HUD 显示「正在做什么」）= 当前回合用户指令截断摘要，后端 turn 上下文现成，不新增工具参数（好默认）。

### 4.3 单写者仲裁（用户拍板：单写，多读自由）

单一鼠标资源的消费者分配：**读（截屏）自由并发；写（鼠标/键盘注入）同一时刻只允许一个会话**。

- **空闲先到先得**：Free 时首个写请求立即获得令牌，无需用户参与。
- **持有粒度=回合**：持有者当前回合持续操作则持有；回合结束自动归还 Free。防长期霸占，也免去多数场景的用户仲裁。归还钩子挂 `cleanup()`/`on_loop_exit`（emitter.rs，注释保证「无论正常/panic/被取消」全退出路径必经——评审 B6；只挂 happy path 会令异常终止后令牌吊死到过期）。归还带归属检查（仅 `Held(本会话)` 才清，防与手动授予竞态互踩）；同钩子把 queue 中本会话的等待者摘除（死会话占队列）。
- **占用中后来者排队**：入 queue，HUD 呈现名单；**切换权归用户**（HUD 队列点选「授予」→ 指定会话获得令牌；原持有者不中止——在飞原子步完成后于下一个 gate 步 park，并作为普通排队者重新入队尾，评审 B9：不入队即丢失唤醒）。不自动抢占——agent 无权自夺（项目安全哲学同源）。
- **排队=取消感知的 park，无固定超时**（评审 B3/B10 修订）：等待令牌与暂停挂起同机制——select { 令牌授予, 通道状态变化, 对话取消 }，不产生错误。原设计的「60s 超时→报错→模型 wait 重试」会让排队 agent 被 doom_detect 按家族签名连败终止（穿插的 wait 成功清不掉同前缀连败），且超时与授予存在竞态（报了错却持有令牌）；正常释放由「回合粒度自动归还」保证，异常吊死由「持有者活性回收」兜底（见下）。HUD 队列可视化 + 用户手动切换是排队者的体验出口。
- **持有者活性回收**（用户复核 B3 补）：写请求争用令牌时先查 ChatState 注册表——持有者会话已不在流式注册表（回合已结束）而令牌仍 Held = 泄漏令牌，自动回收并按队列授予。活性信号复用现成 `is_conversation_streaming` 的注册表，只读查询、不触碰持有者会话本身——**不做跨会话操作**（用户拍板：所有权归属清晰，只有持有者会话自身与用户能作用于一个回合）。
- **队列情报对模型可见**：写操作结果摘要附带通道快照（持有者 + 其会话是否流式中 + 队列名单）。工具 park 时模型处于挂起、无法在挂起中自行放弃——「排不到就友善终止并告知用户」的决策点只能发生在它下一次思考时，靠这份快照支撑（先做读操作 / 直接告知用户占用中）。
- **排队深水区后置**（用户拍板）：持有者活着但卡死（活性为真、令牌不还）、排队公平性/优先级/让行协议等场景足够多，待独立一轮设计讨论；v1 只做活性回收 + 可见性 + 用户手动切换。
- **人类优先（最高优先级）**：见 4.5。

**实现接缝（gate 化，读写分家——评审 B11 修订）**：input.rs / keyboard.rs 的操作序列重构为「原子步」循环，每步前 `gate_write(conv).await`；截图工具入口调 `gate_read(conv).await`。

```
gate_read(conv)：通道 Off→Err(已关闭) → paused→park
gate_write(conv)：gate_read 前置 → human_active→park
               → 令牌（Free→授予本会话；Held(本会话)→过；否则入队 park 等待授予）
park 统一形态：select { 状态恢复, 对话取消(→Err 取消), 通道关闭(→Err 已关闭) }
```

**park 的对话取消感知是硬要求**（评审 B1，高危）：本项目取消是协作式（stop → token.cancel()，工具 future 不会被外部中断），gate 若只 await watch——用户暂停通道 → agent 挂起 → 用户点「停止生成」→ 工具仍挂在 park、ChatState 注册表不注销 → 会话锁死「已有在途生成任务」，audit 史上 P0「会话卡死」模式重演。`wait` 工具（keyboard.rs）已示范正确形态：select `wait_for_cancel`。

原子步定义：一次不可再分的注入组合（click 的 down+up / press_key 的完整组合序列 / **drag 整条插值从头到按住释放**）。步与步之间才可 park 挂起。

### 4.4 暂停与终止（用户拍板：双键，播放器语义）

**暂停**（通道保持、授权保持、读写全部挂起）：
- `paused=true` → 所有 gate park（读写都停）。
- park 实现为 watch channel await + **对话取消 select**（B1：停止生成必须能打断挂起——取消语义与现有「停止生成」一致，统一终止逻辑不动）——恢复时无感继续，agent 回合静静停在工具调用处。
- 兜底已核实：内置工具调用无超时（仅授权等待有 30min 兜底）；前端 60s 静默看门狗走 `is_conversation_streaming` 真相确认，不会误判挂起中的回合（既有不变式复用）。
- 前端两窗（主窗+HUD）显示暂停态（L2 状态上屏）。

**终止**（关通道）：
- Off 转换时：所有 park 中的 gate、排队中的等待者统一收到家族错误 `screen 通道已关闭: 用户结束了屏幕共享`。
- **回合不取消**——错误作为 tool_result 正常返回，agent 自己消化并用自己的话告知用户「任务因共享结束而终止」。这是与「连带杀回合」的关键区别（用户拍板）。
- HUD 与主窗开关同步熄灭；attached 清空；ScreenState 的 CaptureMeta 保留（坐标基准与通道生命周期解耦，布局 revalidate 自防过期）。

### 4.5 人类优先仲裁（用户鼠标永远可用）

**技术事实**：Windows 桌面栈只有一个系统光标，真·多指针做不到（无内核驱动）。实现的是**人类优先避让**：

- **判别机制**：WH_MOUSE_LL + WH_KEYBOARD_LL 低级钩子（专线程 + 消息泵；windows-sys feature 已备，零新依赖）。注入事件带 `LLMHF_INJECTED` 标志——钩子回调只登记**非注入**事件的时间戳，即「物理输入 = 人类在场」。钩子回调内不做任何重活（系统会摘除超时钩子，也绝不能在回调里 SendInput 同类事件——重入死锁）；仲裁全在工具协程侧消费时间戳。
- **仲裁行为**：`human_active`（2s 去抖窗口内有物理输入）→ 所有写 gate park；正在执行的原子序列每步插值后**非阻塞检查**该旗标，命中 → 立即安全收尾（释放按住的键/按钮）+ 中止序列 + 返回家族错误 `screen 用户抢占: 检测到用户正在使用鼠标/键盘，操作已中止并释放输入`（诚实反馈，模型可稍后重试）。
- 用户闲置 2s → human_active 复位，park 的写操作恢复。
- 体验等价：用户随时可夺回鼠标，agent 永远避让；代价是共享光标会被 agent 挪动（HUD「当前操作中」+ 红边框正是为此提供可解释性）。

### 4.6 无自动过期（v1，用户复核拍板）

评审轮曾定「5min 无活动自动收回」；用户复核后**移除**，理由成立：

- **当前引擎下，空闲通道 = 零屏幕访问**。工具只在回合内执行，回合只由用户消息（或其直接延续：自动续写/钩子）触发——没有用户活动就没有截图/操作发生，过期能保护的东西实际不存在。
- 安全兜底换成**常驻可见性**：红边框全屏常亮 + HUD 常显开启时长，用户随时一键终止；泄漏令牌由「持有者活性回收」（4.3）兜底。
- **重引条件（写死在案）**：一旦「跨回合后台驱动」（待机自主监控，批次⑤+）落地——agent 可在无用户发起的情况下持续操作——时间边界必须随之回来（那是真正的挂机遗弃风险）。届时再设计，勿提前。

### 4.7 HUD（用户拍板：点击穿透边框为必做件）

**工具栏窗**（通道 On 时从 Rust 动态创建，Off 销毁）：

- 参数：`label="screen-hud"`、`decorations=false`、`always_on_top=true`、`skip_taskbar=true`、`resizable=false`，尺寸约 420×44，位于 `hud_monitor` 指定显示器顶部居中（monitors() 数据已有）。
- 前端：新增路由 `/screen-hud`（独立小页，不入主布局）；**独立最小 capability 文件**（仅 `core:event:default`，windows 数组只含 `screen-hud`）——评审 A7/B13 修正：Tauri v2 ACL 只 gate 插件命令/非 local origin，自定义 `screen_channel_*` 命令本就全局可 invoke；HUD 真正缺的是 core:event（漏配=收不到事件，而非白屏）；且勿把 screen-hud 塞进 default.json 的 windows 数组——那会让它继承 main 的 opener/dialog 全套权限，过授权。
- 显示器切换：HUD 自带 ◀ ▶（运行时状态，不进设置页——状态上屏哲学）。
- **写操作避让**（评审 B7）：HUD 常驻顶部居中会挡住 agent 对其正下方 UI（浏览器标签栏/菜单栏）的点击——SendInput 命中测试先打到 topmost 的 HUD。令牌持有者原子序列执行期间，HUD 收缩为角部微条并 `set_ignore_cursor_events(true)`（控制暂不可点，人类仲裁让出后恢复）；窗级穿透无法逐控件，收缩是唯一干净解。
- **手动收起/展开**（用户复核 B7 补）：HUD 自带收起按钮——收起为角部微条（**终止键保留**，一键可达），点击展开回全功能条。与写操作避让的自动收缩共用同一收缩形态，两路触发、一套渲染。
- **内容清单 v1**（用户拍板：先按讨论整理，后续边做边补）：

| 区块 | 内容 |
|---|---|
| 控制 | 暂停/恢复（双键之一）、终止（双键之二）、收起/展开（用户复核 B7 补） |
| 状态 | 开启时长（无自动过期，见 4.6） |
| 使用者 | 附着会话数 + 名单；当前令牌持有者（agent 名 + purpose 摘要） |
| 等待 | 写令牌排队名单 + 用户手动「授予」切换 |
| 冲突 | 「检测到你在使用鼠标——agent 已让出」状态条（human_active 可视化） |
| 显示器 | ◀ ▶ 切换 HUD 所在屏 |
| 成本（可选） | 通道累计截图张数 |

**红边框窗**（必做）：

- `label="screen-frame"`、`transparent=true`、`decorations=false`、`always_on_top=true`、`set_ignore_cursor_events(true)`（点击穿透），单窗口覆盖整个虚拟桌面（含负坐标多屏），3px 警示红描边（复用 --ip-danger 系语义）。
- 语义：「正在共享」的全局可见信号（腾讯会议/Zoom 同款语言）；边框铺满所有显示器——通道授权的是整个桌面（agent 可截任意屏），诚实信号。
- 已知限制：独占全屏游戏可能盖过 topmost（与 GDI 全屏盲区同源，接受）。

**HUD 自身会进截图**（顶部一条）：**接受不裁**——模型看到的与用户看到的一致是诚实原则，裁掉反而造成坐标错位。截图 note 的 crop 建议已提示模型可避开。

### 4.8 全屏游戏尽力识别（用户拍板：能识别则友好提示，识别不到不勉强）

两条启发式，命中即报，不命中沉默：

1. **帧均匀性**：捕获结果采样若干点，唯一颜色数/方差低于阈值——降档处理（评审 B12：全屏暗色视频/黑底真内容同样低方差，直接报错误伤）。默认 warning note 附在截图结果上；仅「严格单色 + 连续 2 次命中」才升级为家族错误 `screen 捕获失败: 画面连续全黑/纯色——目标可能处于全屏独占或受保护内容（DRM）`。
2. **前景窗口铺满显示器**：rect 恰好等于某显示器尺寸且无标题栏（无边框全屏特征）→ 截图结果附 warning note（不阻断）：「前景疑似全屏应用，捕获可能异常」。

### 4.9 事件协议与命令

后端 → 全窗广播（`Emitter::emit` 对所有 webview 窗生效；前端→后端必须 invoke，既有 Tauri v2 作用域不变式）：

- `screen:channel-state`（单一全量事件，低频广播）：`{status, paused, opened_at, hud_monitor, attached[], holder|null, queue[], human_active, screenshot_count}`——主窗开关与 HUD 同源渲染。
- `screen:channel-closed`：`{reason: "user"}`（终止归因；无自动过期，见 4.6）。

前端 invoke 命令（挂 commands 层，走现有 bridge）：

- `screen_channel_open / screen_channel_stop / screen_channel_pause / screen_channel_resume`
- `screen_channel_set_hud_monitor(index)` / `screen_channel_grant(conv_id)`（手动切换令牌）/ `screen_channel_detach(conv_id)`
- `get_screen_channel_state`（开关/HUD 初始化拉取）

### 4.10 错误家族增补（doom_detect 首行前缀纪律）

| 家族前缀 | 触发 |
|---|---|
| `screen 通道已暂停` | gate park 被终止打断时的变体说明（正常 park 不产生错误） |
| `screen 通道已关闭` | 终止后一切读写（用户结束共享） |
| `screen 用户抢占` | 原子序列中检测到物理输入，安全收尾后中止 |

三段式纪律（发生了什么+为什么+怎么办）照旧；错误即行为契约照旧。

### 4.11 引擎接缝与不变式

- **授权短路点**：`tool_executor` 的授权决策处（`check_authorization_with_session` 之后）加通道检查——工具 ∈ computer-use 家族（10 个内置名固定集合，注册于 register_builtin；外部 MCP 工具带 `t{idx}_` 前缀无冲突）&& 通道 Active && 会话已附着 → 决策覆盖为 `AuthorizationDecision::Allow`。**只吃 Confirm，不碰 Deny**（评审 A1：无条件覆盖会越过显式永久拒绝）。**不改各工具自身**，授权逻辑单点收敛；会话分层授权（mark_tool_authorized 等）与通道并存。未附着会话的首次使用走「加入共享」Confirm（见 4.1）。
- **gate 不改 ScreenState**：坐标基准仍 per-conv（各会话发送尺寸不同），revalidate 仍防布局过期；通道只管授权/仲裁/可见性。通道 Off 后 meta 保留。
- **截图压缩双钩不变**：通道提高截图频率，keep_last_k=3 的 token 治理更重要了，语义不变。
- **60s 看门狗**：park 挂起期间无事件——前端超时触发后 invoke `is_conversation_streaming` 确认注册表仍在跑 → 不误判（既有不变式天然覆盖，无需新增心跳）。可选增强：gate park/resume 各发一次 chat:processing（若加，走稳定 stage 词表流程）。
- **事件日志**（评审 A4/B4 修订）：`append_event` 硬编码 turn_id 且 session_events 要求真实会话 FK——通道开/关由用户命令触发、无 conv 无 turn，伪造 turn_id 会毒化 reconcile 的 turn 锚点分组（幻影 turn → 假 diff）。**v1 决策：通道生命周期事件走 tracing 日志（日志页可见），不进 session_events**；会话内可归因的事实（工具调用/被暂停打断的错误）已随 tool_result 天然落库。未来若需轨迹级审计，按新 kind 三件套纪律（emit + derive skip + 前端 RowKind）补，勿绕词表。`wait` 工具语义不变（select 取消令牌），通道暂停独立于对话取消。

### 4.12 提交切分（每步 cargo 绿 + 可独立手测）

1. **通道态+授权短路+request_screen_session**：无 HUD，聊天头开关（简单 toggle 按钮）验证授权上收与开关语义。
2. **HUD 工具栏窗+红边框+capabilities+事件协议**：多窗口基建落地。
3. **单写令牌+排队+gate 重构**：input/keyboard 序列原子步化，读写工具过 gate。
4. **人类优先仲裁**：LL hook 线程+human_active+抢占安全收尾。
5. **全屏识别+文案终准**：帧均匀性检查、描述/错误文案收口。

依赖序：1→3→4 严格；2 可与 3 并行；5 收尾。批次③ 真机手测不阻塞（引擎语义不因通道改变，通道 Off = 现状）。

步骤 3/4 各带**无头状态机测试先行**（评审 B14；批次③ 注入式先例：FakeBackend / FakeInputBackend）：gate FSM 七路 select（Off / 暂停 / human / 令牌授予 / 排队 / 取消 / 关闭）、令牌生命周期（回合归还带归属检查 / 手动切换入队尾 / 异常路径摘队列 / 活性回收）、human 去抖注入时间戳。

### 4.13 风险与开放问题

- **LL 全局钩子**：系统级开销小但存在；安全软件可能将「装钩子的进程+注入输入」组合视为可疑——与既有 SendInput 风险同源，接受并文档化。
- **透明点击穿透窗**：个别 GPU/旧驱动的合成兼容性——真机走查项；退化路径：边框窗失败则仅保留 HUD 工具栏（功能不损，信号减弱）。
- **park 挂起 × 既有机制交互**（评审 B3 重核）：预算熔断 token 级（挂起期无消耗，不触发）；stuck_detect 抓轮指纹（挂起期无新轮，不触发）；doom_detect 抓签名连败——park 路径不产生错误故不触发，**排队改为无超时 park 后此风险整体消失**（原「60s 超时→报错→重试」方案会让排队者被 doom_detect 六连败终止，已废弃）。「用户暂停后忘记恢复」无兜底——HUD 常显暂停态即提示本体（过期计时已冻结，见 4.4/4.6）。
- **审批卡 scope 档对 request_screen_session 无意义**：三档都不表达「开通道」——前端特判为二键（开启/拒绝），后端忽略 scope 值（`#[serde(default)]` 已可缺省）。
- **HUD purpose 空闲语义**：附着但无当前回合的会话显示「（空闲）」，不保留陈旧回合摘要。
- **排队深水区（开放）**：持有者活着但卡死（活性查询为真、令牌不还）目前只有用户手动切换一个出口；排队公平性/优先级/让行协议等待独立一轮设计讨论（用户 2026-08-28 复核拍板后置）。
- **多显示器 HUD 跟随**：v1 指定显示器静态放置；「活动屏跟随」（agent 正在操作的显示器）列为后续可选。
- **通道与多主窗**：当前单主窗架构；若未来多主窗，开关状态以通道单例为准（已天然支持）。

## 设计评审记录

- **2026-08-28 对抗评审一轮**（独立评审员对照代码库逐条核实 + 缺陷挖掘）：A 类 7 条——6 属实（授权接缝可插入 / 内置工具无超时 / 事件广播全窗 / windows-sys features 零新依赖 / chat_state 挂起期在册 / 多窗 API 就绪），A4、A7 部分属实已修订（session-events 无 turn 容器 → 通道事件走 tracing 不落库；Tauri v2 ACL 实际只 gate 插件命令 → 独立最小 capability）。B 类 14 条全部吸收——高危 3 条：**B1** gate park 必须带对话取消 select（否则暂停后「停止生成」失灵、会话锁死，audit 史 P0 模式）；**B2** 暂停冻结过期与排队计时（否则暂停与 5min 自动收回互杀）；**B3** 排队超时×doom_detect 连败终止（改为无超时 park，同时消除 B10 授予竞态）。中低 11 条：Deny 不被短路（A1）、附着授权面收紧为「开启者免确认/他者首用加入确认」（B5）、归还钩子挂 cleanup+归属检查+队列摘除（B6）、HUD 写操作避让收缩（B7）、物理输入刷新过期（B8）、手动授予后原持有者入队尾（B9）、gate 读写分家（B11）、帧均匀性降为 warning 档（B12）、独立 capability（B13）、状态机测试先行（B14）、purpose 空闲语义。
- **2026-08-28 用户复核二轮**：B1 维持（取消=粗暴终止，统一终止逻辑暂不重构）；**B2 反转——移除 5min 自动过期**（空闲通道=零屏幕访问，常驻可见性即兜底；后台自主模式落地时时间边界必须重引，见 4.6）；B3 补两件轻量兜底（持有者活性回收走 ChatState 只读查询 + 队列情报对模型可见），**跨会话操作明确不做**、排队深水区独立设计后置；B7 补 HUD 手动收起/展开（终止键常驻）；B5 维持（手测后再调）。

## 待办与里程碑

- [ ] 批次③ 真机手测——自动化冒烟已绿（坐标链偏差 0/三路捕获/GDI 字节序契约），剩余人工面：中文输入手感/拖拽/点击滚轮/组合键/wait 中断 + 多显示器与 DPI 缩放（本机单屏 96 DPI，需换硬件）——不阻塞批次④ 开发
- [ ] 批次④ 按提交切分 1-5 推进
- [ ] 批次④ 真机走查（HUD 多窗/透明边框/钩子仲裁/抢占时序）
