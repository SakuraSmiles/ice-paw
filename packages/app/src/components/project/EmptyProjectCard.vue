<script setup lang="ts">
// EmptyProjectCard — 项目管理空状态
//
// 职责：
//   - 在 filteredProjects.length === 0 时显示
//   - 4 张模板推荐卡（软件开发 / 研究调研 / 内容创作 / 学习助手）
//   - 右下 ghost link「浏览全部模板 →」
//
// 设计要点：
//   - 根占满整个 grid（grid-column: 1 / -1）
//   - 入场：grid stagger 60ms × 4（template card 各延迟）
//   - paw 图标使用内联 SVG（暂时本地定义；待 §3 PawBrandMark 落地后可改为引用）
//
// 注：原 spec 推荐 <PawBrandMark :size="64" :animated="false" />，
//   因 Task 1 / 3 的 PawBrandMark 组件尚未合并，本组件使用同结构 inline SVG。
//   待 PawBrandMark 落地后可重构为：
//     import PawBrandMark from '../common/PawBrandMark.vue'
//     <PawBrandMark :size="64" :animated="false" />

import { useRouter } from "vue-router";
import { ArrowRight, Code, Search, PenTool, GraduationCap } from "lucide-vue-next";

const router = useRouter();

interface TemplateCard {
  key: string;
  icon: typeof Code;
  title: string;
  desc: string;
  /** accent 类名（用于渐变色） */
  accentClass: string;
}

const templates: TemplateCard[] = [
  {
    key: "code",
    icon: Code,
    title: "软件开发",
    desc: "主 Agent + 审查 Agent",
    accentClass: "template-card--ice",
  },
  {
    key: "research",
    icon: Search,
    title: "研究调研",
    desc: "搜索 + 总结 + 写作",
    accentClass: "template-card--aurora",
  },
  {
    key: "writing",
    icon: PenTool,
    title: "内容创作",
    desc: "大纲 + 文案 + 润色",
    accentClass: "template-card--violet",
  },
  {
    key: "learning",
    icon: GraduationCap,
    title: "学习助手",
    desc: "讲解 + 测验 + 反馈",
    accentClass: "template-card--ember",
  },
];

function browseAll(): void {
  void router.push({ name: "TemplateManager" });
}
</script>

<template>
  <div class="empty-project-card" role="status" aria-live="polite">
    <!-- 头部：paw + 标题 + 副文 -->
    <div class="empty-head">
      <div class="empty-paw-wrap" aria-hidden="true">
        <!-- 内联 paw SVG（待 PawBrandMark 落地后可替换） -->
        <svg
          class="empty-paw"
          viewBox="0 0 32 32"
          xmlns="http://www.w3.org/2000/svg"
        >
          <defs>
            <linearGradient id="emptyPawGrad" x1="0" y1="0" x2="1" y2="1">
              <stop offset="0%" stop-color="var(--ip-primary-400)" />
              <stop offset="100%" stop-color="var(--ip-primary-600)" />
            </linearGradient>
          </defs>
          <rect width="32" height="32" rx="7" fill="url(#emptyPawGrad)" />
          <g transform="translate(4, 4) scale(1.45)" fill="#FFFFFF">
            <ellipse cx="6" cy="6" rx="1.7" ry="2.3" transform="rotate(-25 6 6)" />
            <ellipse cx="11" cy="2.8" rx="1.7" ry="2.3" />
            <ellipse cx="16" cy="6" rx="1.7" ry="2.3" transform="rotate(25 16 6)" />
            <path d="M 4.5 12.5 Q 4.5 9.8, 7.8 9.3 L 14.2 9.3 Q 17.5 9.8, 17.5 12.5 Q 17.5 17.2, 11 18.2 Q 4.5 17.2, 4.5 12.5 Z" />
          </g>
        </svg>
      </div>
      <h3 class="empty-title">
        想从一个<em>模板项目</em>开始？
      </h3>
      <p class="empty-sub">挑一个起点，Agent 团队已预置。</p>
    </div>

    <!-- 模板推荐卡 2x2 -->
    <div class="template-grid">
      <button
        v-for="(tpl, idx) in templates"
        :key="tpl.key"
        :class="['template-card', tpl.accentClass]"
        :style="{ animationDelay: `${idx * 60}ms` }"
        type="button"
        tabindex="0"
      >
        <span class="template-card-icon" aria-hidden="true">
          <component :is="tpl.icon" :size="22" :stroke-width="2" />
        </span>
        <span class="template-card-title">{{ tpl.title }}</span>
        <span class="template-card-desc">{{ tpl.desc }}</span>
      </button>
    </div>

    <!-- 右下 ghost link -->
    <button class="empty-link" type="button" @click="browseAll">
      浏览全部模板
      <ArrowRight :size="14" aria-hidden="true" />
    </button>
  </div>
</template>

<style scoped>
/* ============================================================
 * 根容器：占满整个 grid
 * ============================================================ */
.empty-project-card {
  grid-column: 1 / -1;
  padding: var(--ip-spacing-12) var(--ip-spacing-6);
  background: var(--ip-color-bg-elevated);
  border: 1px dashed var(--ip-color-border-default);
  border-radius: var(--ip-radius-3xl);
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  gap: var(--ip-spacing-6);
  animation: ip-empty-state-in var(--ip-duration-page) var(--ip-ease-emphasized) both;
}

/* ============================================================
 * 头部
 * ============================================================ */
.empty-head {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--ip-spacing-3);
  max-width: 540px;
}

.empty-paw-wrap {
  width: 80px;
  height: 80px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: var(--ip-primary-50);
  border-radius: var(--ip-radius-full);
  padding: var(--ip-spacing-3);
  flex-shrink: 0;
}

.empty-paw {
  width: 64px;
  height: 64px;
  display: block;
}

.empty-title {
  margin: 0;
  font-family: var(--ip-font-display);
  font-size: 20px;
  font-weight: var(--ip-font-weight-semibold);
  line-height: 1.4;
  color: var(--ip-color-text-primary);
  letter-spacing: -0.01em;
  text-wrap: balance;
}

.empty-title em {
  font-style: italic;
  color: var(--ip-primary-600);
}

.empty-sub {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-tertiary);
  max-width: 480px;
}

/* ============================================================
 * 模板推荐卡 2×2 grid
 * ============================================================ */
.template-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--ip-spacing-3);
  width: 100%;
  max-width: 560px;
}

.template-card {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: var(--ip-spacing-2);
  padding: var(--ip-spacing-4);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  cursor: pointer;
  text-align: left;
  font-family: inherit;
  color: inherit;
  transition:
    transform var(--ip-duration-base) var(--ip-ease-out),
    box-shadow var(--ip-duration-base) var(--ip-ease-out),
    border-color var(--ip-duration-base) var(--ip-ease-out);
  opacity: 0;
  animation: ip-empty-state-in var(--ip-duration-page) var(--ip-ease-emphasized) forwards;
}

.template-card:hover {
  transform: translateY(-1px);
  box-shadow: var(--ip-shadow-md);
  border-color: var(--ip-primary-400);
}

.template-card:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.template-card-icon {
  width: 36px;
  height: 36px;
  border-radius: var(--ip-radius-md);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--ip-white);
  flex-shrink: 0;
}

/* 4 个 accent 渐变色（与 spec §4.3.4 对齐） */
.template-card--ice .template-card-icon {
  background: linear-gradient(135deg, #6FA1D6, #3565A8); /* primary-400 → primary-600 */
}
.template-card--aurora .template-card-icon {
  background: linear-gradient(135deg, #3BAF7A, #237A58);
}
.template-card--violet .template-card-icon {
  background: linear-gradient(135deg, #8b7ed1, #5B4DA8);
}
.template-card--ember .template-card-icon {
  background: linear-gradient(135deg, #D4A03C, #9E6F1E);
}

.template-card-title {
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  line-height: 1.3;
}

.template-card-desc {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.4;
}

/* ============================================================
 * 浏览全部模板 link
 * ============================================================ */
.empty-link {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: var(--ip-spacing-2) var(--ip-spacing-3);
  background: transparent;
  border: none;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-link);
  cursor: pointer;
  border-radius: var(--ip-radius-md);
  transition:
    background-color var(--ip-duration-base) var(--ip-ease-out),
    color var(--ip-duration-base) var(--ip-ease-out);
}

.empty-link:hover {
  background: var(--ip-color-bg-hover);
  color: var(--ip-primary-700);
}

.empty-link:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* ============================================================
 * 响应式
 * ============================================================ */
@media (max-width: 767px) {
  .empty-project-card {
    padding: var(--ip-spacing-8) var(--ip-spacing-4);
  }

  .template-grid {
    grid-template-columns: 1fr;
  }

  .empty-title {
    font-size: 18px;
  }
}
</style>