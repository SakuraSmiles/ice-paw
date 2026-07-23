// 项目色点工具：项目名 → 5 个 accent 之一（稳定派生）
//
// 职责：
//   - 从项目名稳定哈希到 5 个 accent 之一（glacier / aurora / ember / violet / moss）
//   - 与 agentAvatar.ts 的 colorFromName 风格一致（djb2 变体哈希 + 取模）
//
// 设计要点：
//   - 哈希函数复用 agentAvatar 的 hashName 模式，保证算法一致性
//   - 不引入新颜色变量；5 个 accent 映射到 --card-accent / --card-soft（ProjectCard 局部）
//   - 排序稳定：相同 name 永远映射到同一 accent

// ============================================================================
// 类型定义
// ============================================================================

/** 项目色点身份（5 个 brand accent 之一） */
export type ProjectAccent = "glacier" | "aurora" | "ember" | "violet" | "moss";

/** 所有 accent 列表（按顺序遍历时使用） */
const ACCENTS: ProjectAccent[] = ["glacier", "aurora", "ember", "violet", "moss"];

// ============================================================================
// 哈希 + 取模
// ============================================================================

/**
 * 从项目名稳定派生一个 accent。
 *
 * 算法：djb2 哈希 + 绝对值 + 取模 5 → accent 索引。
 *
 * @param name 项目名称
 * @returns ProjectAccent（5 个之一）
 */
export function accentFromName(name: string): ProjectAccent {
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = ((hash << 5) - hash + name.charCodeAt(i)) | 0;
  }
  const idx = Math.abs(hash) % ACCENTS.length;
  // idx 必定在 [0, ACCENTS.length) 范围内，! 用于收窄 TS strict 下可能的 undefined
  return ACCENTS[idx]!;
}