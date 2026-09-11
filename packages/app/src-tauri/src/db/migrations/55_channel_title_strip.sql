-- 频道标题回归项目名本体（2026-09-11 拍板：频道身份改由会话头 tag 徽章呈现，
-- 「 · 频道」后缀不再拼进标题字符串）。存量剥除 ensure_channel 曾写入的后缀：
-- 后缀恰 5 字符（空格 + · + 空格 + 频 + 道；· 为 U+00B7，与 Rust 写入字节一致）；
-- SQLite length()/substr() 对文本值按字符数计，LIKE 默认 BINARY 逐字节匹配。
UPDATE conversations
SET title = substr(title, 1, length(title) - 5)
WHERE kind = 'channel' AND title LIKE '% · 频道';
