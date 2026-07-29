// IcePaw 演示数据种子脚本（Node.js）
// 用 Node.js 内置 sqlite 模块写入演示数据
// 运行: node --experimental-sqlite seed_demo.mjs

import { DatabaseSync } from 'node:sqlite';
import { fileURLToPath } from 'url';
import path from 'path';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const dbPath = process.argv[2] || path.join(__dirname, '..', '..', '..', '..', '..', '..', '..', 'Users', 'dabai', 'AppData', 'Roaming', 'com.icepaw.app', 'ice-paw.db');

console.log('DB path:', dbPath);
const db = new DatabaseSync(dbPath);

function run(sql, params = []) {
  try {
    db.prepare(sql).run(...params);
  } catch (e) {
    if (!e.message.includes('UNIQUE constraint')) throw e;
  }
}

// 1. 创建 Agent
run(`INSERT OR IGNORE INTO agents (id, name, provider, model, system_prompt, api_key_ref, temperature, max_tokens)
  VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
  ['demo-agent-001', 'Demo 助手', 'anthropic', 'claude-sonnet-4-20250514', '你是 Demo 助手，帮助用户演示工具调用功能。', 'demo-key', 0.7, 4096]
);

// 2. 创建 Conversation
run(`INSERT OR IGNORE INTO conversations (id, agent_id, title, created_at, updated_at)
  VALUES (?, ?, ?, datetime('now', '-1 hour'), datetime('now'))`,
  ['demo-conv-001', 'demo-agent-001', '🧪 工具调用演示']
);

// 3. 消息：场景1 — read_file
run(`INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
  VALUES (?, ?, ?, ?, ?, ?, datetime('now', '-55 minutes'), ?)`,
  ['demo-msg-user-1', 'demo-conv-001', 'user', '请读取项目根目录的 Cargo.toml 文件', '[]', null, null]
);

run(`INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
  VALUES (?, ?, ?, ?, ?, ?, datetime('now', '-54 minutes'), ?)`,
  ['demo-msg-asst-1', 'demo-conv-001', 'assistant',
    '好的，我来读取 Cargo.toml 文件。',
    JSON.stringify([
      { type: "text", text: "好的，我来读取 Cargo.toml 文件。" },
      { type: "tool_use", id: "toolu_demo_1", name: "read_file", input: JSON.stringify({ path: "Cargo.toml", max_bytes: 1048576 }) },
      { type: "tool_result", tool_use_id: "toolu_demo_1", content: JSON.stringify({ path: "Cargo.toml", size: 874, content: "[package]\nname = \"ice-paw\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ntauri = { version = \"2\", features = [] }\nserde = { version = \"1\", features = [\"derive\"] }\nserde_json = \"1\"\nsqlx = { version = \"0.8\", features = [\"runtime-tokio\", \"sqlite\"] }\ntokio = { version = \"1\", features = [\"full\"] }" }), is_error: false }
    ]),
    120, 'claude-sonnet-4-20250514'
  ]
);

// 4. 消息：场景2 — thinking + tool
run(`INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
  VALUES (?, ?, ?, ?, ?, ?, datetime('now', '-49 minutes'), ?)`,
  ['demo-msg-asst-2', 'demo-conv-001', 'assistant',
    '让我先查看项目目录结构，了解整体布局。',
    JSON.stringify([
      { type: "thinking", thinking: "用户想了解项目的代码结构。我需要先查看顶层目录结构，了解项目使用了哪些技术栈和模块划分。从目录结构可以初步判断项目架构。" },
      { type: "text", text: "让我先查看项目目录结构，了解整体布局。" },
      { type: "tool_use", id: "toolu_demo_2", name: "list_directory", input: JSON.stringify({ path: "." }) },
      { type: "tool_result", tool_use_id: "toolu_demo_2", content: JSON.stringify([
        { name: "src", is_dir: true, size: null },
        { name: "Cargo.toml", is_dir: false, size: 874 },
        { name: "README.md", is_dir: false, size: 2456 },
        { name: "target", is_dir: true, size: null }
      ]), is_error: false }
    ]),
    180, 'claude-sonnet-4-20250514'
  ]
);

// 5. 消息：场景3 — 双工具并行
run(`INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
  VALUES (?, ?, ?, ?, ?, ?, datetime('now', '-44 minutes'), ?)`,
  ['demo-msg-asst-3', 'demo-conv-001', 'assistant',
    '我来同时读取这两个文件。',
    JSON.stringify([
      { type: "text", text: "我来同时读取这两个文件。" },
      { type: "tool_use", id: "toolu_demo_3a", name: "read_file", input: JSON.stringify({ path: "src/main.rs", max_bytes: 1048576 }) },
      { type: "tool_use", id: "toolu_demo_3b", name: "read_file", input: JSON.stringify({ path: "src/lib.rs", max_bytes: 1048576 }) },
      { type: "tool_result", tool_use_id: "toolu_demo_3a", content: JSON.stringify({ path: "src/main.rs", size: 420, content: "fn main() {\n    println!(\"Hello, IcePaw!\");\n}" }), is_error: false },
      { type: "tool_result", tool_use_id: "toolu_demo_3b", content: JSON.stringify({ path: "src/lib.rs", size: 1150, content: "pub mod commands;\npub mod context;\npub mod crypto;\npub mod db;\npub mod error;\npub mod harness;\npub mod infra;\npub mod loop;\n\n#[cfg_attr(mobile, tauri::mobile_entry_point)]\npub fn run() {\n    // ...\n}" }), is_error: false }
    ]),
    210, 'claude-sonnet-4-20250514'
  ]
);

// 6. 消息：场景4 — 工具错误
run(`INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
  VALUES (?, ?, ?, ?, ?, ?, datetime('now', '-39 minutes'), ?)`,
  ['demo-msg-asst-4', 'demo-conv-001', 'assistant',
    '抱歉，出于安全原因无法读取该文件。',
    JSON.stringify([
      { type: "tool_use", id: "toolu_demo_4", name: "read_file", input: JSON.stringify({ path: "/etc/shadow", max_bytes: 1024 }) },
      { type: "tool_result", tool_use_id: "toolu_demo_4", content: "出于安全原因，不允许读取系统虚拟文件系统", is_error: true },
      { type: "text", text: "抱歉，出于安全原因无法读取该文件。" }
    ]),
    85, 'claude-sonnet-4-20250514'
  ]
);

// 7. 消息：场景5 — 只有工具、无文本回复
run(`INSERT OR IGNORE INTO messages (id, conversation_id, role, content, content_blocks, token_count, created_at, model)
  VALUES (?, ?, ?, ?, ?, ?, datetime('now', '-34 minutes'), ?)`,
  ['demo-msg-asst-5', 'demo-conv-001', 'assistant',
    '',
    JSON.stringify([
      { type: "tool_use", id: "toolu_demo_5", name: "list_directory", input: JSON.stringify({ path: "src" }) },
      { type: "tool_result", tool_use_id: "toolu_demo_5", content: JSON.stringify([
        { name: "main.rs", is_dir: false, size: 420 },
        { name: "lib.rs", is_dir: false, size: 1150 },
        { name: "commands", is_dir: true, size: null },
        { name: "context", is_dir: true, size: null },
        { name: "db", is_dir: true, size: null },
        { name: "harness", is_dir: true, size: null }
      ]), is_error: false }
    ]),
    60, 'claude-sonnet-4-20250514'
  ]
);

db.close();
console.log('✅ 演示数据写入完成！请在应用中切换到 "🧪 工具调用演示" 对话查看。');
