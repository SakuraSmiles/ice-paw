/**
 * 项目图标名 → Lucide 组件映射。
 *
 * icon 字段存储 lucide 图标名（字符串），本模块统一映射到 Vue 组件。
 * 用法：
 *   import { resolveProjectIcon } from "@/utils/projectIconMap";
 *   const Comp = resolveProjectIcon(proj.icon);
 *   <component :is="Comp" :size="16" />
 */

import {
  Clipboard,
  Folder,
  FlaskConical,
  Wrench,
  Book,
  Palette,
  Settings,
  Snowflake,
  type LucideIcon,
} from "lucide-vue-next";

const ICON_MAP: Record<string, LucideIcon> = {
  clipboard: Clipboard,
  folder: Folder,
  "flask-conical": FlaskConical,
  wrench: Wrench,
  book: Book,
  palette: Palette,
  settings: Settings,
  snowflake: Snowflake,
};

/** 所有可选的项目图标名（用于图标选择器） */
export const PROJECT_ICON_OPTIONS = Object.keys(ICON_MAP) as string[];

/**
 * 将项目图标名映射到 Lucide 组件。
 * 找不到时降级为 Folder。
 */
export function resolveProjectIcon(name?: string | null): LucideIcon {
  if (!name) return Folder;
  return ICON_MAP[name] ?? Folder;
}
