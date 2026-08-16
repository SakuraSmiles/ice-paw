# 贡献指南

## 技术栈

| 层级 | 选型 |
|------|------|
| 桌面壳 | Tauri 2 |
| 前端框架 | Vue 3（`<script setup>` SFC） |
| 前端语言 | TypeScript 5.6 |
| 状态管理 | Pinia 3 |
| 路由 | vue-router 4 |
| 构建工具 | Vite 6 |
| 后端语言 | Rust 2021 edition |
| 数据库 | SQLite + sqlx（异步、连接池、WAL 模式） |
| 密码学 | tauri-plugin-stronghold + blake2b |
| 测试（前端） | Vitest + happy-dom + @vue/test-utils |
| 代码规范 | ESLint 10 (flat config) + Prettier 3 |

## 开发环境

| 工具 | 版本 | 说明 |
|------|------|------|
| Node.js | **18+** | 推荐 20 LTS |
| pnpm | **9+** | monorepo 包管理 |
| Rust | **1.75+** | Tauri 2 MSRV 1.77；推荐 rustup stable |
| OS | Windows / macOS / Linux | 需 WebView（Win: WebView2 / macOS: WKWebView / Linux: WebKitGTK） |

> 平台工具链细节见 [Tauri 官方文档](https://v2.tauri.app/start/prerequisites/)

## 快速开始

```bash
# 安装依赖
pnpm install

# 启动开发模式（Vite + Tauri 窗口）
pnpm tauri dev
```

首次启动会自动：
1. 创建 SQLite 数据库（含所有 migration）
2. 初始化 Stronghold vault
3. 注册所有 Tauri Commands

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
│   └── ui/                           # 共享 UI 样式（CSS tokens）
├── docs/
│   └── architecture.md               # 系统架构文档
├── memory/                            # 项目记忆与迭代计划
└── pnpm-workspace.yaml
```

> 详细架构见 [docs/architecture.md](docs/architecture.md)

## 常用命令

| 命令 | 作用 |
|------|------|
| `pnpm dev` | 仅 Vite 开发服务器（端口 1420） |
| `pnpm tauri dev` | Tauri 开发环境（前端 HMR + Rust 热编译） |
| `pnpm build` | 类型检查 + 生产构建 |
| `pnpm typecheck` | TypeScript 类型检查 |
| `pnpm lint` | ESLint |
| `pnpm test` | 前端 Vitest 测试 |
| `pnpm test:watch` | Vitest watch 模式 |

## 测试

### 前端

```bash
pnpm test          # 51 tests（utils/stores/api）
pnpm test:watch    # watch 模式
```

### Rust

```bash
cd packages/app/src-tauri

# 需显式传 SODIUM_LIB_DIR（或 cd 到 src-tauri 让 Cargo 自动读取 .cargo/config.toml）
SODIUM_LIB_DIR="path/to/sodium-prebuilt/libsodium/x64/Release/v143/static" \
SODIUM_STATIC=true cargo test --lib   # 712 tests

cargo clippy                           # Lint
```

> SODIUM_LIB_DIR 详见 [cargo check 环境](memory/cargo-check-env.md)

## 代码规范

- **TypeScript**：`strict: true`，零 `any` 隐式逃逸
- **Rust**：`cargo clippy` 零告警，`cargo fmt` 格式化
- **命名**：TS camelCase / Rust snake_case，零违规
- **组件**：`<script setup lang="ts">`，`defineProps`/`defineEmits` 全部显式类型
- **Tauri 调用**：所有 `invoke()` 必须通过 `bridge.ts`，业务组件不得直接调用

## API Key 加密

敏感凭据**永不**存 SQLite 明文：

1. 前端通过 `bridge.agents.create()` 投递明文
2. Rust 侧调 `crypto::store_api_key()` 写入 Stronghold vault
3. 数据库 `agents.api_key_ref` 仅存引用 key
4. `Agent` 序列化已用 `#[serde(skip_serializing)]` 屏蔽 `api_key_ref`

Stronghold key 派生：passphrase → blake2b256 → 32 字节 key。

## 数据库

- `foreign_keys = ON`
- `synchronous = NORMAL`
- `journal_mode = WAL`
- 迁移脚本：`src-tauri/src/db/migrations/`（按文件名顺序执行）

## 路线图

- [x] M1-M5：基础架构 + 多 Agent + 工具系统 + 项目空间
- [x] 测试体系（712 Rust + 51 前端）
- [ ] OS keyring 接入替代固定 passphrase
- [ ] 会话搜索 / 导出
- [ ] 前端 E2E 测试（Playwright + Tauri driver）

## 许可

MIT License，详见 [LICENSE](LICENSE)。
