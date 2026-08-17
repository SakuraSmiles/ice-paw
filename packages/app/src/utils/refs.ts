// @ 引用辅助：稳定短码（@会话 / @agent / @消息 的 display 消歧后缀）
//
// 用 FNV-1a 32 位哈希取 4 位数字（% 10000）：同一 id 永远同一短码，跨端不漂移。
// 不用 rowid——SQLite VACUUM 可重排物理行号。碰撞（万分之一级）由名称前缀消歧。
// 后端只认 target_id，短码纯装饰（不参与任何查找）。

/** FNV-1a 32 位 → 4 位数字短码（前导零补齐）。 */
export function shortCode(id: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < id.length; i++) {
    h ^= id.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return String((h >>> 0) % 10000).padStart(4, "0");
}
