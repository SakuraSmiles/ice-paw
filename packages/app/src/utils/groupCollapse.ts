/**
 * 组级收纳（工具折叠 / 思考聚合 / 过程收纳）的纯函数工具（U3-3 第一刀下沉）。
 *
 * 从 ChatMessages.vue 抽出的可独立单测纯函数——补课目标：这些行为原先只能经
 * 组件挂载（mount ChatMessages）间接验证，下沉后直接断言，测试面更窄更稳。
 */

import { formatThinkingMs } from "./format";

/** 思考聚合区的一段思考。key 与 item 内 think-block 展开键同构（msgId + '-h' +
 *  段序），聚合前后展开态互通；durationMs = 段耗时（持久口径，null = 旧消息无）。 */
export interface ThinkSegment {
  key: string;
  text: string;
  durationMs: number | null;
}

/**
 * 展开集键转移（纯函数）：旧键在集合里才转移——返回新集合，不可变语义（勿原地
 * mutate；调用方以返回值替换 ref）。分页前插并组时组键 `grp-<首条id>` 随组头易主，
 * 三层收纳（工具/思考/过程）的展开态按此转移，避免用户手动展开被静默重置回收纳。
 */
export function transferKey<T>(set: ReadonlySet<T>, oldKey: T, newKey: T): Set<T> {
  if (!set.has(oldKey)) return new Set(set);
  const next = new Set(set);
  next.delete(oldKey);
  next.add(newKey);
  return next;
}

/**
 * 思考聚合段的标签（镜像 item 内三态）：块耗时 → 「思考 · 30s」，无耗时 → 只显
 * 「思考」。durationMs 的秒/分换算走 formatThinkingMs 单一真相源。
 */
export function thinkSegLabel(seg: Pick<ThinkSegment, "durationMs">): string {
  return seg.durationMs != null ? "思考 · " + formatThinkingMs(seg.durationMs) : "思考";
}
