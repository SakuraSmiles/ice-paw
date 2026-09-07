/**
 * 工具行级摘要解析器 —— 把工具调用的参数/结果 JSON 解析成单行展示结构。
 *
 * 行形态（2026-09-07 拍板）：
 *   [glyph] 展示名  次级信息 ···(弹性空隙)··· 文件名(可点 reveal) [▸]
 *
 * 数据源：参数 argsJson（文件名位，流式参数到齐即出）+ 结果 JSON（次级信息，
 * 后端字段现成零改动——write_file.backup=null 即新建、edit_file.replacements
 * 计数、docx 各计数，见 plan 数据源速查表）。
 *
 * 返回 null（调用方回退通用行，delegate/plan 卡片先例）：
 *   1. 工具不在 SUMMARY_TOOLS 集合（P1 外降级英文原名 + truncateJson）
 *   2. argsJson parse 失败（流式参数逐字未到齐）或路径字段缺失/畸形
 */

import { formatFileSize } from "./format";
import { toolDisplayName } from "./toolLabels";

/** 工具行摘要（单行渲染所需全部字段） */
export interface ToolLineSummary {
  /** 展示名（词表降级英文原值） */
  display: string;
  /** 次级信息（"" = 不显示；错误态 = "调用失败"） */
  secondary: string;
  /** 文件名位：basename，或 move/copy 的 "旧名 → 新名" */
  fileLabel: string;
  /** hover tooltip（move/copy 为完整双路径） */
  fileTitle: string;
  /** revealItemInDir 目标（move/copy = destination，产物在目标位置） */
  revealPath: string;
}

/** 路径 → basename（兼容 Windows `\` 与 Unix `/` 分隔符混用） */
export function basenameOf(p: string): string {
  const segs = p.split(/[\\/]/).filter(Boolean);
  return segs.length > 0 ? segs[segs.length - 1] : p;
}

/** 路径 → 父目录（含尾分隔符，openPath 可直接用；无分隔符返回 ""） */
export function dirnameOf(p: string): string {
  const idx = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return idx >= 0 ? p.slice(0, idx + 1) : "";
}

/** docx 投影档 → 中文（词表外裸透原值，与 toolLabels 同口径） */
export const DOCX_PROJECTION_LABELS: Record<string, string> = {
  outline: "大纲",
  text: "文本",
  format: "格式",
  ppr: "段落属性",
  headers_footers: "页眉页脚",
  table: "表格",
  tblpr: "表属性",
  styles: "样式档案",
  styledef: "样式定义",
  numbering: "编号",
};

/** 本解析器覆盖的工具集合（P1：文件六件 + docx 四件 + read_file） */
const SUMMARY_TOOLS = new Set([
  "write_file",
  "edit_file",
  "delete_file",
  "move_file",
  "copy_file",
  "create_directory",
  "read_file",
  "inspect_docx",
  "edit_docx",
  "write_docx",
  "validate_docx",
]);

interface ToolResultLike {
  content: string;
  isError: boolean;
}

/** 宽容 parse 结果 JSON（错误态/非 JSON → null，调用方按字段缺失降级） */
function parseResultJson(result?: ToolResultLike | null): Record<string, unknown> | null {
  if (!result || result.isError || !result.content) return null;
  try {
    const o: unknown = JSON.parse(result.content);
    return typeof o === "object" && o !== null ? (o as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

function numOf(o: Record<string, unknown>, key: string): number | null {
  const v = o[key];
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

function strOf(o: Record<string, unknown>, key: string): string | null {
  const v = o[key];
  return typeof v === "string" ? v : null;
}

/** 单路径工具的公共骨架：文件名位 + 次级信息生成器 */
function singlePathSummary(
  display: string,
  path: string,
  result?: ToolResultLike | null,
  secondaryOf?: (r: Record<string, unknown>) => string,
): ToolLineSummary {
  const r = parseResultJson(result);
  return {
    display,
    secondary: result && result.isError ? "调用失败" : r && secondaryOf ? secondaryOf(r) : "",
    fileLabel: basenameOf(path),
    fileTitle: path,
    revealPath: path,
  };
}

/** move/copy 双路径形态：同名（纯移动）只显目标名，改名显 "旧 → 新" */
function dualPathSummary(display: string, source: string, destination: string, secondary: string): ToolLineSummary {
  const srcName = basenameOf(source);
  const dstName = basenameOf(destination);
  return {
    display,
    secondary,
    fileLabel: srcName === dstName ? dstName : `${srcName} → ${dstName}`,
    fileTitle: `${source} → ${destination}`,
    revealPath: destination,
  };
}

/**
 * 解析工具调用为行级摘要。参数畸形/非 P1 工具返回 null（回退通用行）。
 * 结果未到达（流式执行中）→ secondary 为 ""，文件名位先出。
 */
export function summarizeToolCall(
  name: string,
  argsJson: string,
  result?: ToolResultLike | null,
): ToolLineSummary | null {
  if (!SUMMARY_TOOLS.has(name)) return null;
  let args: Record<string, unknown>;
  try {
    const o: unknown = JSON.parse(argsJson);
    if (typeof o !== "object" || o === null) return null;
    args = o as Record<string, unknown>;
  } catch {
    return null; // 流式参数未到齐
  }
  const path = strOf(args, "path");

  switch (name) {
    case "write_file":
      if (!path) return null;
      return singlePathSummary(toolDisplayName(name), path, result, (r) => {
        // write_file 的 backup 是显式 null（新建）/ 字符串（覆盖备份路径）
        const created = !("backup" in r && r.backup != null);
        const bytes = numOf(r, "bytes_written");
        return created ? `新建${bytes != null ? ` · ${formatFileSize(bytes)}` : ""}` : `覆盖${bytes != null ? ` · ${formatFileSize(bytes)}` : ""}`;
      });

    case "edit_file":
      if (!path) return null;
      return singlePathSummary(toolDisplayName(name), path, result, (r) => {
        const n = numOf(r, "replacements");
        return n != null ? `替换 ${n} 处` : "";
      });

    case "delete_file":
      if (!path) return null;
      return singlePathSummary(toolDisplayName(name), path, result, (r) =>
        r.backup != null ? "已删除 · 已备份" : "已删除",
      );

    case "move_file": {
      const source = strOf(args, "source");
      const destination = strOf(args, "destination");
      if (!source || !destination) return null;
      return dualPathSummary(toolDisplayName(name), source, destination, "");
    }

    case "copy_file": {
      const source = strOf(args, "source");
      const destination = strOf(args, "destination");
      if (!source || !destination) return null;
      const r = parseResultJson(result);
      // copy 的 backup = 被覆盖目标的备份；null = 目标新建
      const secondary = result && result.isError ? "调用失败" : r && r.backup != null ? "覆盖" : "";
      return dualPathSummary(toolDisplayName(name), source, destination, secondary);
    }

    case "create_directory":
      if (!path) return null;
      return singlePathSummary(toolDisplayName(name), path, result, () => "已创建");

    case "read_file":
      if (!path) return null;
      return singlePathSummary(toolDisplayName(name), path, result, (r) => {
        const size = numOf(r, "size");
        const lines = numOf(r, "total_lines");
        const parts: string[] = [];
        if (size != null) parts.push(formatFileSize(size));
        if (lines != null) parts.push(`${lines} 行`);
        return parts.join(" · ");
      });

    case "inspect_docx":
      if (!path) return null;
      return singlePathSummary(toolDisplayName(name), path, result, (r) => {
        const proj = strOf(r, "projection");
        const blocks = numOf(r, "total_blocks");
        const parts: string[] = [];
        if (proj) parts.push(DOCX_PROJECTION_LABELS[proj] ?? proj);
        if (blocks != null) parts.push(`${blocks} 块`);
        return parts.join(" · ");
      });

    case "edit_docx":
      if (!path) return null;
      return singlePathSummary(toolDisplayName(name), path, result, (r) => {
        const n = numOf(r, "applied");
        return n != null ? `${n} 处操作` : "";
      });

    case "write_docx":
      if (!path) return null;
      return singlePathSummary(toolDisplayName(name), path, result, (r) => {
        // write_docx 新建时 backup 字段整体缺省（skip_serializing_if），
        // 判据用显式 created 字段——与 write_file 的「显式 null」是两套，勿混
        const created = r.created === true;
        const n = numOf(r, "blocks");
        return `${created ? "新建 · " : ""}${n != null ? `${n} 块` : ""}`.trim() || "";
      });

    case "validate_docx":
      if (!path) return null;
      return singlePathSummary(toolDisplayName(name), path, result, (r) => {
        const total = numOf(r, "total");
        const failed = numOf(r, "failed");
        if (total == null) return "";
        return failed === 0 ? `${total} 项 · 全部通过` : `${total} 项 · ${failed} 项未过`;
      });

    default:
      return null; // SUMMARY_TOOLS 与 switch 对不上（防御，理论不可达）
  }
}
