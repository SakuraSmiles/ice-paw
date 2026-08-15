// composables/useTurnRail.ts
// 轮次导航条数据内核（UX #5）：轮次锚点拉取 + 千轮规模分桶。
//
// 定位是「目录」不是 minimap：分页只加载窗口内消息，未加载页的内容高度
// 不可知（content-visibility 估算也不覆盖未挂载页），按像素比例的 minimap
// 画不出来也不诚实；按轮次索引的目录对任意规模都成立。
// 轮号 = user 消息下标 +1（与轨迹页「第 N 轮」同基准）。

import { ref } from "vue";
import { bridge } from "../api/bridge";
import type { TurnAnchor } from "../types";

/** 轨道最多渲染的 tick 数（超过即聚合）；≈轨道高 / 最小 tick 间距 */
export const RAIL_CAPACITY = 120;

/** 一个 tick：单轮（from==to）或聚合组（from<to，组内连续轮） */
export interface TurnBucket {
  /** 起始轮号（1-based，含） */
  from: number;
  /** 结束轮号（含）；from==to 为单轮 */
  to: number;
  /** 组首轮 user 消息 id（跳转锚点） */
  messageId: string;
  /** 组首轮用户消息预览 */
  preview: string;
  createdAt: string;
}

/**
 * 分桶纯函数：≤capacity 一轮一线；超过按 ceil(N/capacity) 等量连续聚合
 * （末组可能不满）。聚合组的 from<to 让 UI 用更高的 tick 暗示密度。
 */
export function buildTurnBuckets(anchors: TurnAnchor[], capacity: number): TurnBucket[] {
  if (anchors.length === 0) return [];
  if (anchors.length <= capacity) {
    return anchors.map((a, i) => ({
      from: i + 1,
      to: i + 1,
      messageId: a.message_id,
      preview: a.preview,
      createdAt: a.created_at,
    }));
  }
  const groupSize = Math.ceil(anchors.length / capacity);
  const out: TurnBucket[] = [];
  for (let i = 0; i < anchors.length; i += groupSize) {
    const first = anchors[i];
    const lastIdx = Math.min(i + groupSize, anchors.length) - 1;
    out.push({
      from: i + 1,
      to: lastIdx + 1,
      messageId: first.message_id,
      preview: first.preview,
      createdAt: first.created_at,
    });
  }
  return out;
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
