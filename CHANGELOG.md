# Changelog

All notable changes to IcePaw will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Multi-Agent Management
- Support for OpenAI, Anthropic, GLM (Zhipu), DeepSeek, and MiniMax providers via OpenAI-compatible API adapters.
- Custom model per agent with `base_url` and API key configuration.
- Agent template system with preset agents and avatar utilities.
- Per-agent configurable tool permissions (disabled / ask / allow) with authorization confirmation flow.
- Per-agent history window config for context size control.
- Session-level model switching (override agent default per conversation).
- `supports_vision` field on agents, auto-hiding image attachments for text-only models.

#### Conversation System
- Full chat interface with Markdown rendering (code highlighting via highlight.js).
- Conversation management: create, rename, pin, delete, and full-text search across conversations.
- Welcome screen with empty-state prompt cards for new conversations.
- Message grouping by date, with time-bucket separators in chat view.
- Message pagination (infinite scroll) for long conversation history.
- Message copy (click to copy any assistant or user message).
- Input box auto-height with Shift+Enter for newlines.
- Draft persistence — unsent input text survives tab switches.
- External links in messages open in the system browser.

#### Project Spaces
- Project dimension: create, edit, list, and switch between project workspaces.
- Per-project agent configuration and workspace path isolation.
- Project archive (soft-delete) with restore, keeping conversations intact but out of the active view.
- Permanent project delete.
- Inline project title editing on cards.
- Sidebar project switcher with dropdown and management entry.
- Tool context (`workspace`) respects project workspace path, falling back to agent default.

#### MCP Tool System
- Built-in agentic tool suite: file read/write/edit, shell execution, grep/code search, git operations, web fetch.
- External MCP server support via stdio subprocess (configurable scope: global or per-agent).
- Per-agent tool registry — each agent sees only its own enabled tools.
- Tool discovery UI showing available tools per MCP server.
- Tool authorization dialog (confirm tool use before execution).
- `read_agent_config` tool — agents can inspect their own configuration.
- Web fetch tool with default `User-Agent` header.
- Tool status bar showing running/completed tool invocations during streaming.

#### Knowledge Base (RAG v1)
- Single-KB model with SQLite-backed document storage.
- Keyword search repository with relevance scoring.
- Ingestion pipeline: file watcher, Markdown/plain-text parser, embedding indexer.
- `search_kb` tool with `execute_with_context` for context-aware retrieval.
- `read_kb_document` tool for full-document retrieval (no auth required).
- KB document list page in frontend settings.

#### Streaming Chat & Visualization
- Real-time streaming message rendering with token-level updates.
- Thinking (chain-of-thought) visualization collapsible within assistant messages.
- Tool-call visualization with expand/collapse for request and result payloads.
- Intelligent scroll-follow during streaming — respects user scroll position, does not steal focus.
- `finish_reason` display in status bar (stop / length / tool_calls / content_filter).
- Round-state observable: token usage, elapsed time, cached tokens, retry count per round.
- Chat status bar (floating capsule) with expanded detail panel.

#### UI & Design System
- Design token system with light/dark theme support and ice-blue primary palette.
- Component library: Button, Input, Textarea, Modal, Toast, Message, Flex, Container, Avatar, Card, Select, EmptyState, DropdownMenu, Popconfirm, IconButton.
- Global fade transitions (~150ms, restrained use).
- Session-switch opacity transitions.
- Sidebar conversation card: active pulse animation during generation, last-interaction time with auto-refresh.
- Collapsible card pattern with full-card click area for expand/collapse.
- Responsive grid layout for project/agent cards.
- Brand signature components (PawBrandMark, PawTrail, AgentAvatarStack).

#### Security
- API key encryption via Stronghold vault (iota_stronghold + blake2b).
- Path whitelist authority for file and shell tools (agents restricted to workspace directory).
- Tool authorization flow (ask/allow/deny) per agent and tool category.
- Protocol sanitization layer (`sanitize_history`) for defense against history injection.

#### Configuration & Settings
- Global settings page with 5 sections: General, Agents, MCP Servers, Knowledge Base, Logs.
- Per-agent settings: model, base URL, API key, history window, tool permissions, workspace path, timezone.
- MCP server configuration UI with scope selector (global/per-agent) and `{workspace}` placeholder support.
- Timezone configuration (system default or custom IANA timezone).
- Instant-save on all settings changes (no save button).

#### Logging & Diagnostics
- Disk-persistent log viewer in settings (tracing-appender with daily rotation).
- Structured logging across Rust backend (context, harness, MCP, DB layers).
- CI pipeline with GitHub Actions (lint, typecheck, test, build).

#### Testing
- 400+ Rust unit tests covering commands, context pipeline, harness, providers, MCP tools, retry, and scoring.
- Integration tests for message repository, provider adapters, and end-to-end memory stage.
- 30+ frontend tests using Vitest + happy-dom for stores, utilities, and API bridge layers.
- Mock agent command infrastructure for deterministic chat loop testing.

### Changed

- Refactored chat subsystem from a monolithic 1568-line `chat_cmd.rs` into 6 focused modules: protocol, cleanup, error, context pipeline, loop engine, and orchestration.
- Migrated LLM providers to a unified `harness/provider/` adapter architecture (OpenAI + Anthropic with shared trait).
- Retry logic replaced with type-safe `RetryState` state machine (was string-based).
- Loop budget constants centralized into `LoopBudget` struct with configurable token/round limits.
- Tool permissions moved from conversation-level to agent-level configuration.
- Context assembly refactored to trait-based `Pipeline` architecture with pluggable stages.
- MCP server manager refactored with per-agent server pool, auto scope derivation, and child-process environment isolation.
- Tool result persistence refactored to per-round independent persistence with startup migration.
- Settings page restructured from flat form to card-based in-place editing.
- Global "more" actions (delete, etc.) moved into a `...` overflow menu to reduce visual clutter.
- Confirm modals de-colored (no more red) for consistent, calm UI.
- Sidebar conversation list auto-loads and recovers last active session on app start.
- Minimum window width increased to 900px for better layout.

### Fixed

- Conversation permanently stuck in "Generating" state after certain error/retry conditions.
- Switching conversations mid-stream losing already-rendered content — now preserves partial output.
- Sidebar conversation card time now reflects last interaction time, not creation time, and refreshes every minute.
- `finish_reason` leaking across conversations (cross-session state contamination).
- LLM errors incorrectly showing "Manually stopped" instead of the actual error message.
- Silent unhandled promise rejections in UI operations — now surfacing user-visible error feedback.
- MCP `notifications/initialized` handshake warning with external stdio servers.
- Per-agent MCP servers incorrectly starting globally — now filtered by scope on create/update/restart.
- Per-agent tool list showing wrong tool hints for unconfigured servers.
- Table rendering in Markdown: `display: block` on `<table>` caused header/body separation.
- User message bubbles appearing white-on-white + assistant bubbles lacking background in dark mode.
- New conversation race condition where store's `loadFor` overwrote a freshly created conversation.
- OpenAI-compatible `base_url` path segment handling (smart path join for `/v1` suffix).
- `miniMax` 400 error due to incorrect `tool_result` nesting in assistant messages.
- Session switch flash showing stale messages from previous conversation.
- Scroll-follow breaking after session-switch fade transitions.
- Vite alias resolution for ESM module project configuration.
- CI disk space exhaustion from debug/release build artifacts.
- Various Clippy warnings and ESLint issues resolved across codebase.

## [0.1.0] — Unreleased (Development Preview)

The current working version. All features listed above are included. This version
is under active development and has not yet had an official release.

The `v0.2.0-rc1` tag in the repository served as an internal milestone marker
(July 2026) for the completion of the multi-agent adapter architecture and
GLM-5.2 integration, but no formal release was cut.
