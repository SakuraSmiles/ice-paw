// composables/useTurnRail.ts
// 轮次导航条数据内核（UX #5 v2：定容滑动窗口）。
//
// 定位是「目录」不是 minimap：分页只加载窗口内消息，未加载页的内容高度
// 不可知，按轮次索引的目录对任意规模（几千轮）都成立。v1 曾用「全量目录 +
// 超容聚合」（一轮可能代表 25 轮）；v2 改「定容窗口 + 当前轮居中」——
// 轨道恒定 ≤ RAIL_WINDOW 格、真实一轮一线，当前视位轮居中锚定，
// 窗口外靠边缘省略号（整窗翻页）/ 轨道滚轮（半窗步进）到达。
// 轮号与轨迹页「第 N 轮」同基准（后端 list_turn_anchors 已排除
// tool_result 占位行，= distinct turn_id）。

import { ref } from "vue";
import { bridge } from "../api/bridge";
import type { TurnAnchor } from "../types";

/** 轨道窗口容量：任意会话规模下轨道最多同时渲染的 tick 数（≈轨道高 / 20px） */
export const RAIL_WINDOW = 13;

/** 一个 tick = 一轮（真实用户消息） */
export interface TurnTick {
  /** 全局轮号（1-based，与轨迹页同基准） */
  turn: number;
  messageId: string;
  preview: string;
  createdAt: string;
}

/** 窗口切片结果：可见 tick + 全局位置指示 */
export interface TurnWindow {
  ticks: TurnTick[];
  /** 窗口首格的全局轮号（1-based） */
  from: number;
  /** 会话总轮数 */
  total: number;
  /** 窗口上方还有轮次（渲染顶部省略号） */
  hasPrev: boolean;
  /** 窗口下方还有轮次（渲染底部省略号） */
  hasNext: boolean;
}

/**
 * 自动窗口起点纯函数：activeTurn 居中（前偏 ⌊size/2⌋ 格），边界钳制。
 * activeTurn 未知（初始未检出）→ 末窗：会话打开时视口在底部跟随最新。
 */
export function autoWindowStart(total: number, activeTurn: number | null, size: number): number {
  if (total <= 0) return 1;
  const maxStart = Math.max(1, total - size + 1);
  if (activeTurn === null) return maxStart;
  return Math.min(maxStart, Math.max(1, activeTurn - Math.floor(size / 2)));
}

/**
 * 窗口切片纯函数：锚点全量 + 窗口起点 → 可见 tick 与边缘指示。
 * `from` 越界时向内钳制（size > total 时恒为 1）。
 */
export function buildTurnWindow(anchors: TurnAnchor[], from: number, size: number): TurnWindow {
  const total = anchors.length;
  const maxStart = Math.max(1, total - size + 1);
  const start = Math.min(Math.max(1, from), maxStart);
  const ticks: TurnTick[] = [];
  for (let i = start - 1; i < Math.min(start - 1 + size, total); i++) {
    ticks.push({
      turn: i + 1,
      messageId: anchors[i].message_id,
      preview: anchors[i].preview,
      createdAt: anchors[i].created_at,
    });
  }
  return { ticks, from: start, total, hasPrev: start > 1, hasNext: start - 1 + size < total };
}

/** 锚点拉取：按会话切换/新轮到达重拉（全量轻量行，幂等） */
export function useTurnRail() {
  const anchors = ref<TurnAnchor[]>([]);

  async function loadAnchors(conversationId: string | null) {
    if (!conversationId) {
      anchors.value = [];
      return;
    }
    try {
      anchors.value = await bridge.trajectory.turnAnchors(conversationId);
    } catch (err) {
      // 拉取失败静默：导航条隐藏（空锚点），下次触发点重试
      console.warn("[turn-rail] 锚点拉取失败", err);
      anchors.value = [];
    }
  }

  return { anchors, loadAnchors };
}
