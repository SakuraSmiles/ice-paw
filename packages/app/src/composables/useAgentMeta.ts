// Agent 元数据管理 composable
//
// 职责：
//   - 读写 Agent 前端附加信息（头像、描述、推荐词）到 localStorage
//   - 优先从 localStorage 读取已保存的 meta（创建时从模板写入）
//   - 降级派生：无 localStorage 数据时，从 Agent.name 派生缩写/颜色、从 system_prompt 提取描述
//
// 存储键格式：`icepaw.agentMeta.${agentId}`
//
// 设计要点：
//   - 无 emoji（v2 严格要求），视觉标识通过 Lucide 图标 + 字母缩写色块实现
//   - 所有降级逻辑内置空值防御，不会因缺失字段而报错
//
// 配套：
//   - src/data/agentTemplates.ts — 模板数据（创建时写入 meta）
//   - src/utils/agentAvatar.ts — 字母缩写 + 颜色哈希工具

import type { Component } from "vue";
import { avatarFromName, initialsFromName } from "../utils/agentAvatar";
import type { Agent } from "../types";

// ============================================================================
// 类型定义
// ============================================================================

/** Agent 前端元数据（持久化到 localStorage） */
export interface AgentMeta {
  /** 缩写字母（如 "通"、"CA"） */
  avatarText: string;
  /** 头像背景色（十六进制） */
  avatarColor: string;
  /** 头像前景色（用于浅色场景） */
  avatarFg?: string;
  /** Lucide 图标组件（有模板时存在） */
  icon?: Component;
  /** 一句话角色描述 */
  description: string;
  /** 推荐开场白（0-4 条） */
  promptChips: string[];
}

/** localStorage 存储的完整 meta（含序列化安全字段） */
export interface AgentMetaFull extends AgentMeta {
  /** 存储时间戳（ISO 8601） */
  storedAt: string;
}

// ============================================================================
// localStorage 读写
// ============================================================================

const LS_PREFIX = "icepaw.agentMeta.";

/**
 * 从 localStorage 读取 Agent meta
 * @param agentId Agent ID
 * @returns 解析后的 AgentMeta，不存在或解析失败返回 null
 */
function readLS(agentId: string): AgentMetaFull | null {
  try {
    const raw = localStorage.getItem(LS_PREFIX + agentId);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as AgentMetaFull;
    return parsed;
  } catch {
    return null;
  }
}

/**
 * 将 Agent meta 写入 localStorage
 * @param agentId Agent ID
 * @param meta 元数据
 */
function writeLS(agentId: string, meta: AgentMeta): void {
  try {
    const full: AgentMetaFull = {
      ...meta,
      storedAt: new Date().toISOString(),
    };
    localStorage.setItem(LS_PREFIX + agentId, JSON.stringify(full));
  } catch {
    // localStorage 满 / 不可用 → 静默降级（不影响核心功能）
  }
}

// ============================================================================
// 降级派生工具
// ============================================================================

/**
 * 从 system_prompt 提取第一行作为描述
 * @param prompt system_prompt 文本（可能为空或 undefined）
 * @returns 一句话描述，无内容返回空字符串
 */
export function extractDescription(prompt: string | null | undefined): string {
  if (!prompt || !prompt.trim()) return "";
  // 取第一行，截断到 60 字符
  const firstLine = prompt.trim().split("\n")[0] ?? "";
  if (firstLine.length <= 60) return firstLine;
  return firstLine.slice(0, 57) + "...";
}

/**
 * 根据 system_prompt 关键词派生推荐词（降级方案）
 * 当 localStorage 中没有 meta.promptChips 时，通过关键词匹配生成推荐词。
 * 带空值防御：输入为空时返回空数组。
 * @param prompt system_prompt 文本（可能为空或 undefined）
 * @returns 推荐词数组（0-4 条）
 */
export function deriveChipsFromPrompt(
  prompt: string | null | undefined,
): string[] {
  if (!prompt || !prompt.trim()) return [];

  const text = prompt.trim().toLowerCase();

  // 关键词 → 推荐词映射（按优先级）
  if (/代码|编程|code|program|debug|review/.test(text)) {
    return [
      "帮我写一段 Python 批量重命名脚本",
      "解释这段代码的时间复杂度",
      "Review 这段代码并指出问题",
      "推荐一个项目的目录结构",
    ];
  }
  if (/翻译|translat|language|语言/.test(text)) {
    return [
      "把这段中文翻译成英文",
      "帮我把这篇英文摘要翻译成中文",
      "检查这段英文的语法",
      "将这段话翻译成日语",
    ];
  }
  if (/写作|文案|write|copywriting|营销/.test(text)) {
    return [
      "帮我写一个咖啡品牌的小红书文案",
      "给一款智能手表写 3 条广告语",
      "写一段产品发布会开场白",
      "帮我起 5 个公众号标题",
    ];
  }
  if (/论文|学术|paper|academic|润色|polish/.test(text)) {
    return [
      "帮我润色这段论文摘要",
      "检查这段文字的学术规范性",
      "帮我改进论文的过渡句",
      "把这段口语化表达改为学术语言",
    ];
  }
  if (/创意|头脑风暴|brainstorm|idea/.test(text)) {
    return [
      "给我 10 个手机 App 的创业点子",
      "如何用 AI 改造传统餐饮业？",
      "帮我想一个团建活动的创意",
      "给我一些副业赚钱的思路",
    ];
  }
  if (/导师|教学|tutor|teach|学习|辅导/.test(text)) {
    return [
      "用大白话解释什么是区块链",
      "帮我理解 Transformer 的注意力机制",
      "解释一下复利的威力",
      "什么是薛定谔的猫？",
    ];
  }
  if (/数据|分析|analys|图表|统计/.test(text)) {
    return [
      "帮我分析这组销售数据的趋势",
      "推荐一个适合时间序列的图表类型",
      "解释什么是 A/B 测试",
      "如何判断两个变量的相关性？",
    ];
  }

  // 无匹配关键词：返回通用推荐词
  return [
    "用一段话总结今天的新闻",
    "帮我写一封请假邮件",
    "推荐一部周末看的电影",
    "解释什么是量子计算",
  ];
}

// ============================================================================
// composable
// ============================================================================

/**
 * Agent 元数据管理 composable
 *
 * 用法：
 *   const { getMeta, setMeta, getFullMeta } = useAgentMeta();
 *   const meta = getMeta(agent);
 */
export function useAgentMeta() {
  /**
   * 获取 Agent 元数据（优先 localStorage，降级派生）
   * @param agent Agent 实体
   * @returns AgentMeta（保证所有字段有值）
   */
  function getMeta(agent: Agent): AgentMeta {
    // 1. 尝试 localStorage
    const stored = readLS(agent.id);
    if (stored) {
      return {
        avatarText: stored.avatarText ?? initialsFromName(agent.name),
        avatarColor: stored.avatarColor ?? "#6366f1",
        avatarFg: stored.avatarFg,
        icon: stored.icon,
        description: stored.description ?? extractDescription(agent.system_prompt),
        promptChips: stored.promptChips ?? deriveChipsFromPrompt(agent.system_prompt),
      };
    }

    // 2. 降级派生
    const avatar = avatarFromName(agent.name);
    return {
      avatarText: avatar.text,
      avatarColor: avatar.color,
      avatarFg: avatar.fg,
      icon: undefined,
      description: extractDescription(agent.system_prompt),
      promptChips: deriveChipsFromPrompt(agent.system_prompt),
    };
  }

  /**
   * 写入 Agent 元数据到 localStorage
   * @param agentId Agent ID
   * @param meta 要持久化的元数据
   */
  function setMeta(agentId: string, meta: AgentMeta): void {
    writeLS(agentId, meta);
  }

  /**
   * 获取完整 meta（含 storedAt 时间戳）
   * @param id Agent ID 或 Agent 实体
   * @returns 完整 meta 或 null
   */
  function getFullMeta(id: string | Agent): AgentMetaFull | null {
    const agentId = typeof id === "string" ? id : id.id;
    return readLS(agentId);
  }

  /**
   * 删除 Agent 的本地元数据（删除 Agent 时同步清理）
   * @param id Agent ID 或 Agent 实体
   */
  function removeMeta(id: string | Agent): void {
    const agentId = typeof id === "string" ? id : id.id;
    try {
      localStorage.removeItem(LS_PREFIX + agentId);
    } catch {
      // localStorage 不可用时静默忽略
    }
  }

  return { getMeta, setMeta, getFullMeta, removeMeta };
}
