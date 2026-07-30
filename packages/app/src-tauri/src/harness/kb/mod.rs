//! `kb` — RAG 知识库摄入管道（parser / indexer / watcher）
//!
//! 把磁盘上的 markdown 文件解析为结构化索引写入 `kb_document` 表，
//! 供 search_kb 工具检索。v1 纯 agentic 检索，无向量/切块。
//!
//! - `parser`：单文件 md → 结构化字段（title/summary/tags）+ content_hash
//! - `indexer`：扫描 KB 目录 → 增量 upsert `kb_document`（按 content_hash）
//! - `watcher`：notify 监听目录变更 → 触发增量索引（启动集成见 lib.rs setup）

pub mod ensure;
pub mod indexer;
pub mod parser;
pub mod watcher;
