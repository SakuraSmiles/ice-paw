// 图片文件处理 composable
//
// 职责：
//   - 将 ImagePicker.vue 中的文件校验 + FileReader 逻辑提取为可复用 composable
//   - 供 ImagePicker（文件选择）、ChatInput（粘贴）、ChatPage（拖拽）共享
//
// 导出：
//   - useImageFiles() → { processFiles }
//   - 常量 ACCEPT_MIMES, MAX_FILE_SIZE, MAX_COUNT
//   - 类型 ImageItem（从 ImagePicker re-export 保持一致）

import { useToast } from "./useToast";

/** 组件对外的图片条目 */
export interface ImageItem {
  /** 裸 base64 字符串（不含 `data:image/...;base64,` 前缀） */
  data: string;
  /** MIME 类型，例如 `image/png` */
  media_type: string;
  /** 完整的 data URL（含前缀，仅用于 `<img src>` 预览） */
  preview: string;
  /** 原文件名（UI 展示用，旧调用方可能未填） */
  fileName?: string;
}

/** 接受的文件 MIME 列表（与 Rust 侧白名单 + input accept 属性对齐） */
export const ACCEPT_MIMES = ["image/png", "image/jpeg", "image/gif", "image/webp"] as const;

/** accept 属性字符串（逗号分隔，用于 <input accept>） */
export const ACCEPT_ATTR = ACCEPT_MIMES.join(",");

/** 单张最大字节数（5MB） */
export const MAX_FILE_SIZE = 5 * 1024 * 1024;

/** 总数上限（与 Rust 侧校验一致） */
export const MAX_COUNT = 20;

/**
 * 图片文件处理 composable。
 *
 * @param currentImages 获取当前已有图片数量的函数（用于计算剩余 slots）
 * @param onUpdate 图片列表更新回调
 * @returns processFiles 函数
 */
export function useImageFiles(
  currentImages: () => ImageItem[],
  onUpdate: (images: ImageItem[]) => void,
) {
  const toast = useToast();

  /**
   * 处理一批文件：校验大小/MIME → 读为 data URL → 拆出 base64 + media_type → 追加到列表。
   * 超出上限的文件自动截取并提示。
   */
  async function processFiles(files: File[]): Promise<void> {
    const current = currentImages();
    const slots = MAX_COUNT - current.length;
    if (slots <= 0) {
      toast.warning(`最多 ${MAX_COUNT} 张图片`);
      return;
    }
    const toProcess = files.slice(0, slots);
    if (files.length > slots) {
      toast.warning(`超过上限，已截取前 ${slots} 张`);
    }

    const additions: ImageItem[] = [];
    for (const f of toProcess) {
      // 大小预校验
      if (f.size > MAX_FILE_SIZE) {
        toast.error(`图片「${f.name || "未命名"}」超过 5MB，已跳过`);
        continue;
      }
      // MIME 预校验
      if (!f.type || !ACCEPT_MIMES.includes(f.type as (typeof ACCEPT_MIMES)[number])) {
        toast.error(`不支持的图片格式：${f.type || "未知"}，仅支持 png/jpeg/gif/webp`);
        continue;
      }

      try {
        const dataUrl = await readAsDataURL(f);
        const { base64, mediaType } = splitDataUrl(dataUrl, f.type);
        additions.push({
          data: base64,
          media_type: mediaType,
          preview: dataUrl,
          fileName: f.name || undefined,
        });
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        toast.error(`读取图片失败：${msg}`);
      }
    }

    if (additions.length > 0) {
      onUpdate([...current, ...additions]);
    }
  }

  return { processFiles };
}

/** FileReader.readAsDataURL 封装为 Promise */
function readAsDataURL(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const r = reader.result;
      if (typeof r === "string") resolve(r);
      else reject(new Error("FileReader 返回非字符串"));
    };
    reader.onerror = () => reject(new Error(reader.error?.message ?? "读取失败"));
    reader.readAsDataURL(file);
  });
}

/** 拆出 base64 主段与 media type */
function splitDataUrl(
  dataUrl: string,
  fallbackType: string,
): { base64: string; mediaType: string } {
  const m = /^data:([^;,]+);base64,(.*)$/.exec(dataUrl);
  if (!m || !m[1] || !m[2]) {
    return { base64: dataUrl, mediaType: fallbackType };
  }
  return { base64: m[2], mediaType: m[1] };
}
