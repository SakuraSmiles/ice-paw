# IcePaw

> 桌面端 AI Agent 聊天客户端（本地优先，数据自主可控）

IcePaw 是一款基于 **Tauri 2** 的桌面应用，前端用 **Vue 3 + TypeScript** 构建，后端用 **Rust** 通过 Tauri Commands 暴露能力。所有对话历史、Agent 配置都落本地 SQLite，敏感凭据（LLM Provider 的 API Key）通过 **Stronghold vault** 加密存储。

## 特性

- **多 Agent 管理**：每个 Agent 独立配置 provider、model、system_prompt、temperature、max_tokens
- **多会话**：每个 Agent 下可建任意多个 conversation，置顶 / 重命名 / 删除
- **消息持久化**：基于 SQLite + sqlx，连接池 + WAL 模式
- **API Key 加密**：Rust 侧通过 Stronghold（snapshot 落 `app_data_dir/stronghold.hold`），前端业务层永远拿不到明文
- **前端封装统一入口**：`src/api/bridge.ts` 是 invoke 唯一出口，业务组件禁止直接 `invoke(...)`
- **TypeScript 严格模式**：`strict: true` + `noUnusedLocals` + `noUnusedParameters`，零 any 隐式逃逸

## 技术栈

| 层级       | 选型                                            |
| ---------- | ----------------------------------------------- |
| 桌面壳     | Tauri 2                                         |
| 前端框架   | Vue 3（`<script setup>` SFC）                   |
| 前端语言   | TypeScript 5.6                                   |
| 状态管理   | Pinia 3                                         |
| 路由       | vue-router 5                                    |
| 构建工具   | Vite 6                                          |
| 后端语言   | Rust 2021 edition                                |
| 数据库     | SQLite + sqlx（异步、连接池、自动迁移）         |
| 密码学     | tauri-plugin-stronghold（vault 加密）+ blake2b |
| 错误处理   | thiserror + 自定义 `AppError` 序列化穿过 IPC     |
| 代码规范   | ESLint 10 (flat config) + Prettier 3            |

## 开发环境要求

| 工具      | 版本要求       | 说明                                                |
| --------- | -------------- | --------------------------------------------------- |
| Node.js   | **18+**        | 推荐 20 LTS                                        |
| pnpm      | **9+**         | 本项目使用 pnpm 管理依赖                            |
| Rust      | **1.75+**      | Tauri 2 要求 MSRV 1.77；建议用 rustup 安装 stable   |
| OS        | Windows / macOS / Linux | Tauri 需要 WebView（Win: WebView2 / macOS: WKWebView / Linux: WebKitGTK） |

> 平台工具链细节参见 Tauri 官方文档：<https://v2.tauri.app/start/prerequisites/>

## 快速开始

```bash
# 1. 安装依赖
pnpm install

# 2. 启动开发模式（Vite + Tauri 窗口）
pnpm tauri dev
```

首次启动会自动完成：

1. 在 app data 目录创建 SQLite 数据库（若不存在）
2. 执行 `sqlx::migrate!()` 跑迁移脚本（`src-tauri/src/db/migrations/`）
3. 初始化 Stronghold，snapshot 落到 `app_data_dir/stronghold.hold`
4. 注入所有 Tauri Commands 到前端

## 常用脚本

| 命令                | 作用                                                                 |
| ------------------- | -------------------------------------------------------------------- |
| `pnpm dev`          | 仅启动 Vite 开发服务器（不启动 Tauri 窗口），端口 1420               |
| `pnpm tauri dev`    | 完整启动 Tauri 开发环境（前端 HMR + Rust 监听编译）                  |
| `pnpm build`        | 类型检查 + 生产构建，输出到 `dist/`                                  |
| `pnpm preview`      | 预览生产构建产物                                                     |
| `pnpm lint`         | 运行 ESLint                                                           |
| `pnpm lint:fix`     | ESLint 自动修复                                                       |
| `pnpm format`       | Prettier 格式化                                                       |
| `pnpm format:check` | Prettier 检查（不写）                                                 |
| `pnpm test`         | 运行前端 Vitest 测试                                                  |
| `pnpm test:watch`   | Vitest watch 模式                                                     |

### Rust 测试与检查

```bash
# 注意：需要显式传 SODIUM_LIB_DIR（libsodium 预编译库路径）
cd packages/app/src-tauri
SODIUM_LIB_DIR="D:/path/to/sodium-prebuilt/libsodium/x64/Release/v143/static" \
SODIUM_STATIC=true cargo test --lib

# 或 cd 到 src-tauri 目录，Cargo 自动读取 .cargo/config.toml
cd packages/app/src-tauri
cargo test --lib

# Clippy
cargo clippy
```

> 关于 SODIUM_LIB_DIR 的详细说明见 [cargo check 环境](memory/cargo-check-env.md)。

## 项目结构

```
ice-paw/
├── packages/
│   ├── app/                          # 主应用
│   │   ├── src/                      # Vue 前端
│   │   │   ├── api/bridge.ts         # Tauri IPC 统一入口
│   │   │   ├── components/           # Vue 组件（chat/common/agent/mcp/kb/layout）
│   │   │   ├── composables/          # 可复用逻辑
│   │   │   ├── pages/                # 路由页面
│   │   │   ├── stores/               # Pinia 状态管理
│   │   │   ├── types/index.ts        # TypeScript 类型定义
│   │   │   └── utils/time.ts         # 时间工具
│   │   └── src-tauri/                # Rust 后端
│   │       └── src/
│   │           ├── commands/          # Tauri Command 入口
│   │           ├── context/           # LLM 上下文装配 Pipeline
│   │           ├── db/                # 数据库（models/repo/migrations）
│   │           ├── harness/           # 核心运行时（provider/loop/mcp/kb）
│   │           └── infra/             # 基础设施（protocol/cancel）
│   └── ui/                           # 共享 UI 样式
├── docs/
│   └── architecture.md               # 系统架构文档
├── memory/                            # 项目记忆与计划
└── pnpm-workspace.yaml
```

> 详细架构见 [docs/architecture.md](docs/architecture.md)

## 数据存放位置

应用数据目录由 Tauri 通过 `app_data_dir()` 解析，路径规则如下：

| 平台     | 数据库文件                                | Stronghold snapshot                       |
| -------- | ----------------------------------------- | ----------------------------------------- |
| Windows  | `%APPDATA%\com.icepaw.app\ice-paw.db`     | `%APPDATA%\com.icepaw.app\stronghold.hold` |
| Linux    | `~/.local/share/com.icepaw.app/ice-paw.db` | `~/.local/share/com.icepaw.app/stronghold.hold` |
| macOS    | `~/Library/Application Support/com.icepaw.app/ice-paw.db` | 同目录 `stronghold.hold` |

数据库连接选项：

- `foreign_keys = ON`（强制外键约束，级联删除 conversations / messages）
- `synchronous = NORMAL`（性能 / 安全性平衡）
- `journal_mode = WAL`（读写并发）

## 首次启动自动建表

迁移脚本位于 `src-tauri/src/db/migrations/`，由 `sqlx::migrate!()` 在启动时执行。当前只有 `01_init.sql` 一份脚本，初始化三张表：

- `agents` —— Agent 元信息（不含 api_key 明文）
- `conversations` —— 会话（外键 → agents，CASCADE）
- `messages` —— 消息（外键 → conversations，CASCADE）

未来新增表 / 改字段时，只需在 `migrations/` 下追加 `02_xxx.sql` 即可，sqlx 会按文件名顺序执行。

## API Key 加密存储

敏感凭据（LLM Provider 的 API Key）**永不**存进 SQLite。流程如下：

1. 前端在创建 / 轮换 Agent 时通过 `bridge.agents.create()` 或 `bridge.agents.rotateKey()` 投递明文
2. Rust 侧 `commands/agent_cmd.rs` 调用 `crypto::store_api_key()` 写入 Stronghold vault
3. 数据库 `agents.api_key_ref` 仅存引用 key（默认 = `agent_id`）
4. 前端业务层任何位置都拿不到明文——`Agent` 序列化结构已用 `#[serde(skip_serializing)]` 屏蔽

Stronghold snapshot 的加密 key 派生：

- passphrase → blake2b256 → 32 字节 key（与 stronghold 文档推荐的 `KeyProvider::with_passphrase_hashed_blake2b` 等价）
- 这样规避了 `KeyProvider::try_from` 强约束「密码必须 32 字节」的 footgun

## 项目结构

```
ice-paw/
├── src/                        # 前端（Vue 3 + TS）
│   ├── api/
│   │   └── bridge.ts           # invoke 唯一出口
│   ├── components/
│   │   ├── agent/              # Agent 业务组件
│   │   ├── chat/               # 聊天业务组件
│   │   ├── common/             # 通用基础组件
│   │   ├── layout/             # 布局（侧边栏 / 主区）
│   │   └── session/            # 会话列表相关
│   ├── composables/            # 组合式函数
│   ├── pages/                  # 页面级组件（与 router 一一对应）
│   ├── router/                 # vue-router 配置
│   ├── stores/                 # Pinia store
│   ├── types/                  # 业务类型（与 Rust 侧结构对齐）
│   ├── utils/
│   │   └── keychain.ts         # 兼容层（已弃用，业务请走 bridge）
│   ├── App.vue
│   └── main.ts
├── src-tauri/                  # Rust 后端
│   ├── src/
│   │   ├── commands/           # 暴露给前端的 invoke 入口
│   │   │   ├── agent_cmd.rs
│   │   │   ├── conversation_cmd.rs
│   │   │   └── message_cmd.rs
│   │   ├── db/                 # sqlx 连接池 + 迁移 + 仓储
│   │   │   ├── migrations/
│   │   │   ├── models.rs
│   │   │   ├── repo/
│   │   │   └── mod.rs
│   │   ├── crypto.rs           # Stronghold 封装
│   │   ├── error.rs            # 统一错误类型（IPC 序列化）
│   │   ├── lib.rs              # 应用入口（setup、plugin 注册）
│   │   └── main.rs
│   ├── capabilities/
│   │   └── default.json        # Tauri 2 权限声明
│   ├── icons/
│   ├── tauri.conf.json
│   ├── Cargo.toml
│   └── build.rs
├── public/                     # 静态资源
├── eslint.config.js            # ESLint flat config
├── .prettierrc                 # Prettier 配置
├── vite.config.ts
├── tsconfig.json               # 前端 TS 配置（含对 tsconfig.node.json 的引用）
├── tsconfig.node.json          # Node 端 TS 配置（vite.config.ts）
├── package.json
└── pnpm-lock.yaml
```

## 注意事项

### WSL 下 Tauri 窗口无法显示

WSL 容器内**没有 GPU 加速 + 没有原生 X11/Wayland 服务器**，Tauri WebView 进程启动后无法创建窗口。表现：

- `pnpm tauri dev` 在 WSL 里跑会一直卡住，或者 `cargo build` 成功但窗口不出现
- 这是 Tauri 本身在 WSL 下的限制（不只是 IcePaw），跟 WSLg、WebView2 安装与否都无关

**解决**：把仓库 clone / 同步到 **Windows 真机**，在 PowerShell / Windows Terminal 中跑 `pnpm tauri dev`。

WSL 内仅适合做纯前端开发（`pnpm dev` + `pnpm build` + `pnpm lint`），不启动 Tauri 窗口。

### Rust 编译慢

首次 `cargo build` 会拉取并编译 Tauri / sqlx / stronghold 几十个 crate，需要 **5~15 分钟**。后续增量编译会快很多。`Cargo.toml` 已为 `scrypt` dev profile 打开 `opt-level=3` 以缓解。

### 数据库文件迁移 / 备份

直接拷贝 `ice-paw.db` 即可（SQLite 文件级备份），但要确保没有活跃写连接——WAL 模式下建议用 `sqlite3 ice-paw.db ".backup backup.db"`。

## 路线图

- [x] M1：Tauri 2 + Vue 3 + TS + Vite 脚手架
- [x] M2-A：SQLite + sqlx + 迁移
- [x] M2-B：Stronghold 加密 + API Key 安全存储
- [x] M2-C：Pinia + vue-router 集成
- [ ] M3：聊天主页面 + LLM 流式调用
- [ ] M4：Agent 管理 UI
- [ ] M5：会话历史 / 搜索 / 导出
- [ ] M6：OS keyring 接入替代固定 passphrase

## 许可

TBD
