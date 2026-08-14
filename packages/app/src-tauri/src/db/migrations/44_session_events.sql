-- =========================================================================
-- IcePaw 数据库迁移 V44：session_events 事件日志（session-event-log Phase 0）
-- 来源：2026-08-14 锁定的统一 session / 轨迹架构（单一 append-only 事件日志）
-- 说明：
--   会话事件的唯一无损真相源（Phase 0 影子写入，Phase 1 derive-on-read 对账，
--   Phase 2 切唯一真相源后 legacy 多表拼装退役）。
--
--   不变式：append-only、永不 UPDATE/DELETE；唯一可丢 = 流式传输 chunk
--   （组装后 message 即原文且必有事件）；压缩只作用于实时窗口 projection。
--
--   设计要点：
--   - id 用 AUTOINCREMENT：INTEGER PRIMARY KEY 在删除最大 rowid 后可能复用 id，
--     脏化「跨 session 按全局 id 排序」的项目级轨迹序；簿记代价可忽略。
--   - seq 是 per-session 单调序号，由 INSERT 内子查询
--     (SELECT COALESCE(MAX(seq),0)+1 ... WHERE session_id=?) 分配——SQLite
--     单语句原子（写锁下子查询与插入不撕裂）。UNIQUE 索引兜底：未来若出现
--     双写者，第二次 INSERT 以约束错误失败 → 产生「可检测缺口」而非
--     「seq 重复污染回放序」。
--   - message_id 故意不设外键：终止守卫会 DELETE assistant 占位行
--     （cleanup.rs），事件必须活得比行久。
--   - turn_id = user_msg_id：同一 send_message 周期的全部事件共享该分组键。
--   - actor 取 'user' | 'agent:<uuid>'，为多 agent 通道预留，本期恒为会话 agent。
--   - 同 message_id 可出现多条 assistant_message 事件（自动续写全文覆写），
--     回放语义 last-wins（supersede）。
--   - payload 为 JSON（Rust 侧强类型 struct，含 v 版本字段）；附件类事件只存
--     元信息不存正文/字节，防三重冗余。
--
--   Phase 2 已知债：summary_* 事件 payload 的 covered_until_rowid 是 messages
--   物理 rowid，切事件日志为主源后需改为 covered_until_seq。
-- =========================================================================

CREATE TABLE IF NOT EXISTS session_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,  -- 全局单调，永不复用
    session_id  TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,                   -- per-session 单调（回放序）
    kind        TEXT NOT NULL,                      -- 事件类型词表（可扩展，无需 migration）
    actor       TEXT NOT NULL DEFAULT 'user',       -- 'user' | 'agent:<uuid>'
    turn_id     TEXT,                               -- 同一 send_message 分组（= user_msg_id）
    message_id  TEXT,                               -- legacy 行关联（Phase 1 对账用；无 FK，见上）
    payload     TEXT NOT NULL DEFAULT '{}',         -- JSON 强类型 payload
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX idx_session_events_session_seq ON session_events(session_id, seq);
