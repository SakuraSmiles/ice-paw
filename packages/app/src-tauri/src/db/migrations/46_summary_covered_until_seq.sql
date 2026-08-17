-- 摘要锚点 seq 化（S1 Phase 2B 阶段 2）
--
-- 双动机：
-- ① 事件纪元的语义锚——legacy 读路径退役后 covered_until 的参照系应是
--   session_events.seq（per-session 单调、append-only、永不复用），而非
--   messages.rowid（表无 AUTOINCREMENT，行删除后 rowid 可能被新行复用 → 锚点漂移）。
-- ② 与 derive 排序位严格一致：派生消息序按 first_seq（supersede 场景取首现事件
--   seq），锚点必须同取首现 seq，否则会越过中间消息（assistant 首现 seq5、
--   tool_result seq6、覆写 seq7 → 锚点必须是 5）。
--
-- 回填：从既有 covered_until_rowid 反查该消息**首现**消息类事件的 seq。
-- kind 限定三消息类是必要的——tool_execution / attachment_stored 事件也带
-- message_id，限定后才与 first_seq 定义逐字相同。
-- 无事件锚点（pre-Phase-0 会话、backfill 未覆盖）→ NULL → 运行期 rowid 兜底。
--
-- 双写过渡：本列与 covered_until_rowid 并存（写双值、读 seq 优先 rowid 兜底）。

ALTER TABLE messages ADD COLUMN covered_until_seq INTEGER;

UPDATE messages
SET covered_until_seq = (
    SELECT MIN(e.seq)
      FROM session_events e
      JOIN messages m ON m.rowid = messages.covered_until_rowid
     WHERE e.session_id = messages.conversation_id
       AND e.message_id = m.id
       AND e.kind IN ('user_message', 'assistant_message', 'tool_result_message')
)
WHERE covered_until_rowid IS NOT NULL
  AND role = 'system'
  AND instr(content, '[Previous conversation summary]') = 1;
