// Agent 头像工具：名字 → 字母缩写 + 颜色 + 文本/背景配色
//
// 职责：
//   - initialsFromName:  从 Agent 名字派生 1-2 个字符的缩写（中文取首字 / 英文取首字母）
//   - colorFromName:     名字稳定哈希到预设饱和色板
//   - saturatedToBgFg:   把饱和主色转为可读 { bg, fg } 配色对（用于纯文本回退场景）
//
// 设计要点：
//   - 完全不使用 emoji（v2 严格要求）
//   - 颜色映射稳定：相同 name 永远映射到同一颜色（哈希 + 取模）
//   - 颜色饱和度高：保证在浅/深色背景下都可见
//
// 配套：
//   - src/components/common/AgentAvatar.vue — 统一头像渲染组件

// ============================================================================
// 色板（饱和度足够的品牌色，覆盖常见主色调）
// ============================================================================

/**
 * 头像背景色板
 *
 * 选择标准：
 *   - 色值饱和度足够（500 级），浅/深色背景下都可见
 *   - 冷暖色均衡，避免连续两张卡片颜色相近
 *   - 共 12 色，覆盖常见品牌色
 */
export const AVATAR_COLORS: readonly string[] = [
  "#6366f1", // indigo-500
  "#8b5cf6", // violet-500
  "#ec4899", // pink-500
  "#f59e0b", // amber-500
  "#10b981", // emerald-500
  "#06b6d4", // cyan-500
  "#3b82f6", // blue-500
  "#ef4444", // red-500
  "#84cc16", // lime-500
  "#f97316", // orange-500
  "#14b8a6", // teal-500
  "#a855f7", // purple-500
] as const;

// ============================================================================
// 缩写生成
// ============================================================================

/**
 * 从 Agent 名字取缩写（最多 2 个字符）。
 *
 * 规则：
 *   - 中文：取第一个汉字
 *   - 英文：单词之间取首字母大写（最多 2 字母）
 *   - 混合（前缀为英文）：取前 2 字符大写
 *   - 空字符串 / 纯空白：返回 "?"
 *
 * @param name Agent 名称
 * @returns 缩写文本（1-2 字符）
 */
export function initialsFromName(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return "?";

  // 中文名：取第一个汉字
  if (/[\u4e00-\u9fa5]/.test(trimmed[0])) {
    return trimmed[0];
  }

  // 英文 / 拉丁字母名：取前两个单词的首字母大写
  const words = trimmed.split(/\s+/).filter((w) => w.length > 0);
  if (words.length >= 2) {
    return (words[0][0] + words[1][0]).toUpperCase();
  }
  // 单词或单个 token：取前 2 字符大写
  return trimmed.slice(0, 2).toUpperCase();
}

// ============================================================================
// 颜色映射
// ============================================================================

/**
 * 把字符串名字哈希为 32 位整数（djb2 变体）
 *
 * @param name 名字
 * @returns 32 位整数哈希
 */
function hashName(name: string): number {
  let hash = 0;
  for (let i = 0; i < name.length; i++) {
    hash = ((hash << 5) - hash + name.charCodeAt(i)) | 0;
  }
  return hash;
}

/**
 * 根据名字稳定地返回一个饱和色（来自色板）。
 *
 * 算法：djb2 哈希 + 绝对值 + 取模 → 色板索引。
 *
 * @param name Agent 名称
 * @returns 十六进制色值（形如 "#6366f1"）
 */
export function colorFromName(name: string): string {
  const hash = hashName(name);
  const idx = Math.abs(hash) % AVATAR_COLORS.length;
  return AVATAR_COLORS[idx];
}

// ============================================================================
// 颜色转 bg/fg 配对
// ============================================================================

/**
 * 把饱和主色（hex）转换为背景/前景色配对。
 *
 * 用途：当 Agent 头像只能渲染纯文本（无图标）时，需要保证文字在背景色上可读。
 * 算法：把 hex 转 HSL，降低亮度（L 减约 35% → 浅色背景）或大幅降低（L 大幅降低 → 深色背景），
 *       同时根据主色亮度决定前景色（深色 fg 或浅色 fg）。
 *
 * @param color 十六进制色值（如 "#6366f1"）
 * @returns { bg, fg } 配色对
 */
export function saturatedToBgFg(color: string): { bg: string; fg: string } {
  // 解析 hex → RGB
  const trimmed = color.trim();
  if (!/^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/.test(trimmed)) {
    // 非法输入：返回安全默认
    return { bg: "#f3f4f6", fg: "#111827" };
  }

  let hex = trimmed.slice(1);
  if (hex.length === 3) {
    hex = hex
      .split("")
      .map((c) => c + c)
      .join("");
  }

  const r = parseInt(hex.slice(0, 2), 16);
  const g = parseInt(hex.slice(2, 4), 16);
  const b = parseInt(hex.slice(4, 6), 16);

  // RGB → HSL
  const { h, s, l } = rgbToHsl(r, g, b);

  // 根据主色亮度选择策略
  //   - 主色较浅（L > 0.6）→ 直接用主色作 bg，fg 选深色
  //   - 主色较深（L <= 0.6）→ 把背景提到 L=0.92（极浅），fg = 原主色
  let bg: string;
  let fg: string;
  if (l > 0.6) {
    bg = color;
    fg = "#111827"; // gray-900
  } else {
    bg = hslToHex(h, Math.min(s, 0.4), 0.92);
    fg = color;
  }

  return { bg, fg };
}

// ============================================================================
// 一站式：名字 → { text, color, bg, fg }
// ============================================================================

/** 头像信息汇总 */
export interface AvatarInfo {
  /** 缩写文本（1-2 字符） */
  text: string;
  /** 主色（用于深色头像背景，如 64×64 大头像） */
  color: string;
  /** 浅色背景（用于浅色场景，如列表项） */
  bg: string;
  /** 前景色（与 bg 配合） */
  fg: string;
}

/**
 * 一站式：从名字派生完整头像信息。
 *
 * @param name Agent 名称
 * @returns AvatarInfo
 */
export function avatarFromName(name: string): AvatarInfo {
  const color = colorFromName(name);
  const { bg, fg } = saturatedToBgFg(color);
  return {
    text: initialsFromName(name),
    color,
    bg,
    fg,
  };
}

// ============================================================================
// 内部：RGB ↔ HSL ↔ Hex 互转
// ============================================================================

/** RGB (0-255) → HSL (h: 0-360, s/l: 0-1) */
function rgbToHsl(r: number, g: number, b: number): { h: number; s: number; l: number } {
  const rNorm = r / 255;
  const gNorm = g / 255;
  const bNorm = b / 255;
  const max = Math.max(rNorm, gNorm, bNorm);
  const min = Math.min(rNorm, gNorm, bNorm);
  const l = (max + min) / 2;
  let h = 0;
  let s = 0;

  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    switch (max) {
      case rNorm:
        h = (gNorm - bNorm) / d + (gNorm < bNorm ? 6 : 0);
        break;
      case gNorm:
        h = (bNorm - rNorm) / d + 2;
        break;
      case bNorm:
        h = (rNorm - gNorm) / d + 4;
        break;
    }
    h *= 60;
  }
  return { h, s, l };
}

/** HSL (h: 0-360, s/l: 0-1) → hex 字符串 */
function hslToHex(h: number, s: number, l: number): string {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  let r = 0;
  let g = 0;
  let b = 0;

  if (h < 60) {
    r = c;
    g = x;
    b = 0;
  } else if (h < 120) {
    r = x;
    g = c;
    b = 0;
  } else if (h < 180) {
    r = 0;
    g = c;
    b = x;
  } else if (h < 240) {
    r = 0;
    g = x;
    b = c;
  } else if (h < 300) {
    r = x;
    g = 0;
    b = c;
  } else {
    r = c;
    g = 0;
    b = x;
  }

  const toHex = (v: number): string => {
    const n = Math.round((v + m) * 255);
    const clamped = Math.max(0, Math.min(255, n));
    return clamped.toString(16).padStart(2, "0");
  };
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}