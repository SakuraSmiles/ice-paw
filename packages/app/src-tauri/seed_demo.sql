-- IcePaw 演示数据种子脚本
-- 创建一个包含各种工具调用场景的演示对话
-- 运行方式: sqlite3 <db路径> ".read <本文件路径>"

-- 1. 创建演示 Agent
INSERT OR IGNORE INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens)
VALUES ('demo-agent-001', 'Demo 助手', 'anthropic', 'claude-sonnet-4-20250514', '你是 Demo 助手，帮助用户演示工具调用功能。', 'demo-key', 0.7, 4096);

-- 2. 创建演示对话
INSERT OR IGNORE INTO conversations (id, agent_id, title, created_at, updated_at)
VALUES ('demo-conv-001', 'demo-agent-001', '🧪 工具调用演示', datetime('now', '-1 hour'), datetime('now'));

-- 3. 插入消息：场景1 — 简单的 read_file 工具调用
-- 用户消息
INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-user-1', 'demo-conv-001', 'user',
  '请读取项目根目录的 Cargo.toml 文件',
  '[]', NULL, datetime('now', '-55 minutes'), NULL);

-- 助手回复：text + tool_use + tool_result
INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-asst-1', 'demo-conv-001', 'assistant',
  '好的，我来读取 Cargo.toml 文件。',
  '[{"type":"text","text":"好的，我来读取 Cargo.toml 文件。"},{"type":"tool_use","id":"toolu_demo_1","name":"read_file","input":"{\"path\":\"Cargo.toml\",\"max_bytes\":1048576}"},{"type":"tool_result","tool_use_id":"toolu_demo_1","content":"{\"path\":\"Cargo.toml\",\"size\":874,\"content\":\"[package]\\nname = \\\"ice-paw\\\"\\nversion = \\\"0.1.0\\\"\\nedition = \\\"2021\\\"\\n\\n[dependencies]\\ntauri = { version = \\\"2\\\", features = [] }\\nserde = { version = \\\"1\\\", features = [\\\"derive\\\"] }\\nserde_json = \\\"1\\\"\\nsqlx = { version = \\\"0.8\\\", features = [\\\"runtime-tokio\\\", \\\"sqlite\\\"] }\\ntokio = { version = \\\"1\\\", features = [\\\"full\\\"] }\"}","is_error":false}]',
  120, datetime('now', '-54 minutes'), 'claude-sonnet-4-20250514');

-- 助手后续文字回复（不含工具调用的纯文本）
INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-asst-1b', 'demo-conv-001', 'assistant',
  '文件内容已展示如上。项目主要依赖 Tauri 2、Serde、SQLx 和 Tokio。',
  '[{"type":"text","text":"文件内容已展示如上。项目主要依赖 Tauri 2、Serde、SQLx 和 Tokio。"}]',
  45, datetime('now', '-53 minutes'), 'claude-sonnet-4-20250514');

-- 4. 场景2 — 思考过程 + 工具调用
INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-user-2', 'demo-conv-001', 'user',
  '帮我分析一下这个项目的代码结构',
  '[]', NULL, datetime('now', '-50 minutes'), NULL);

INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-asst-2', 'demo-conv-001', 'assistant',
  '让我先查看项目目录结构，了解整体布局。',
  '[{"type":"thinking","thinking":"用户想了解项目的代码结构。我需要先查看顶层目录结构，了解项目使用了哪些技术栈和模块划分。从目录结构可以初步判断项目架构。"},{"type":"text","text":"让我先查看项目目录结构，了解整体布局。"},{"type":"tool_use","id":"toolu_demo_2","name":"list_directory","input":"{\"path\":\".\"}"},{"type":"tool_result","tool_use_id":"toolu_demo_2","content":"[{\"name\":\"src\",\"is_dir\":true,\"size\":null},{\"name\":\"Cargo.toml\",\"is_dir\":false,\"size\":874},{\"name\":\"README.md\",\"is_dir\":false,\"size\":2456},{\"name\":\"target\",\"is_dir\":true,\"size\":null}]","is_error":false}]',
  180, datetime('now', '-49 minutes'), 'claude-sonnet-4-20250514');

INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-asst-2b', 'demo-conv-001', 'assistant',
  '项目顶层目录包含 src/ 源码目录、Cargo.toml 配置、README.md 文档，以及编译输出 target/。src/ 目录下应该有主要的 Rust 源代码。要深入了解可以继续查看子目录。',
  '[{"type":"text","text":"项目顶层目录包含 src/ 源码目录、Cargo.toml 配置、README.md 文档，以及编译输出 target/。src/ 目录下应该有主要的 Rust 源代码。要深入了解可以继续查看子目录。"}]',
  90, datetime('now', '-48 minutes'), 'claude-sonnet-4-20250514');

-- 5. 场景3 — 多个并行工具调用
INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-user-3', 'demo-conv-001', 'user',
  '读取 src/main.rs 和 src/lib.rs 看看入口代码',
  '[]', NULL, datetime('now', '-45 minutes'), NULL);

INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-asst-3', 'demo-conv-001', 'assistant',
  '我来同时读取这两个文件。',
  '[{"type":"text","text":"我来同时读取这两个文件。"},{"type":"tool_use","id":"toolu_demo_3a","name":"read_file","input":"{\"path\":\"src/main.rs\",\"max_bytes\":1048576}"},{"type":"tool_use","id":"toolu_demo_3b","name":"read_file","input":"{\"path\":\"src/lib.rs\",\"max_bytes\":1048576}"},{"type":"tool_result","tool_use_id":"toolu_demo_3a","content":"{\"path\":\"src/main.rs\",\"size\":420,\"content\":\"fn main() {\\n    println!(\\\"Hello, IcePaw!\\\");\\n}\"}","is_error":false},{"type":"tool_result","tool_use_id":"toolu_demo_3b","content":"{\"path\":\"src/lib.rs\",\"size\":1150,\"content\":\"pub mod commands;\\npub mod context;\\npub mod crypto;\\npub mod db;\\npub mod error;\\npub mod harness;\\npub mod infra;\\npub mod loop;\\n\\n#[cfg_attr(mobile, tauri::mobile_entry_point)]\\npub fn run() {\\n    // ...\\n}\"}","is_error":false}]',
  210, datetime('now', '-44 minutes'), 'claude-sonnet-4-20250514');

-- 6. 场景4 — 工具执行错误
INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-user-4', 'demo-conv-001', 'user',
  '读取 /etc/shadow 文件',
  '[]', NULL, datetime('now', '-40 minutes'), NULL);

INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-asst-4', 'demo-conv-001', 'assistant',
  '抱歉，出于安全原因无法读取该文件。',
  '[{"type":"tool_use","id":"toolu_demo_4","name":"read_file","input":"{\"path\":\"/etc/shadow\",\"max_bytes\":1024}"},{"type":"tool_result","tool_use_id":"toolu_demo_4","content":"出于安全原因，不允许读取系统虚拟文件系统","is_error":true},{"type":"text","text":"抱歉，出于安全原因无法读取该文件。"}]',
  85, datetime('now', '-39 minutes'), 'claude-sonnet-4-20250514');

-- 7. 场景5 — list_directory 工具
INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-user-5', 'demo-conv-001', 'user',
  '看看 src 目录下有哪些文件',
  '[]', NULL, datetime('now', '-35 minutes'), NULL);

INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-asst-5', 'demo-conv-001', 'assistant',
  '',
  '[{"type":"tool_use","id":"toolu_demo_5","name":"list_directory","input":"{\"path\":\"src\"}"},{"type":"tool_result","tool_use_id":"toolu_demo_5","content":"[{\"name\":\"main.rs\",\"is_dir\":false,\"size\":420},{\"name\":\"lib.rs\",\"is_dir\":false,\"size\":1150},{\"name\":\"commands\",\"is_dir\":true,\"size\":null},{\"name\":\"context\",\"is_dir\":true,\"size\":null},{\"name\":\"db\",\"is_dir\":true,\"size\":null},{\"name\":\"harness\",\"is_dir\":true,\"size\":null}]","is_error":false}]',
  60, datetime('now', '-34 minutes'), 'claude-sonnet-4-20250514');

INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
VALUES ('demo-msg-asst-5b', 'demo-conv-001', 'assistant',
  'src 目录下包含以下内容：
- main.rs — 程序入口
- lib.rs — 库入口
- commands/ — Tauri 命令模块
- context/ — 上下文模块
- db/ — 数据库模块
- harness/ — 核心调度层',
  '[{"type":"text","text":"src 目录下包含以下内容：\\n- main.rs — 程序入口\\n- lib.rs — 库入口\\n- commands/ — Tauri 命令模块\\n- context/ — 上下文模块  \\n- db/ — 数据库模块\\n- harness/ — 核心调度层"}]',
  95, datetime('now', '-33 minutes'), 'claude-sonnet-4-20250514');
