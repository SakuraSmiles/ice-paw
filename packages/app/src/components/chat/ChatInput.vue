<!--
  ChatInput — 聊天输入框 + 图片/文档附件 + 发送/停止按钮

  行为：
  - Enter 发送，Shift+Enter 换行
  - 图片：按钮选择 / 拖拽 / 粘贴 → base64 ContentBlock（image）
  - 文档（docx/xlsx/xls/pdf）：按钮选择 / 拖拽 / 粘贴 → 后端 materialize 为 Text 块
    （office 是输入模态，LLM 读不了 OOXML 二进制；提取在后端 doc::try_extract）
  - 流式生成中按钮切换为「停止」+ 动画指示器
  - draftText 自动保存（切换会话不丢未发送内容）

  Props: 无（通过 chat store 读写）
  Emits: 无
-->
<script setup lang="ts">
import { computed, watch, nextTick, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { readFile } from "@tauri-apps/plugin-fs";
import { useChatStore } from "../../stores/chat";
import type { ContentBlock } from "../../types";

const chat = useChatStore();
const textareaRef = ref<HTMLTextAreaElement | null>(null);

const input = computed({
  get: () => chat.draftText,
  set: (v: string) => { chat.draftText = v; },
});

watch(() => chat.sending, (sending) => {
  if (!sending) nextTick(() => textareaRef.value?.focus());
});

function autoResize() {
  const el = textareaRef.value;
  if (!el) return;
  el.style.height = "auto";
  el.style.height = Math.min(el.scrollHeight, 200) + "px";
}

// ===== base64 编码（图片/文件共用）=====
// 分块转换避免主线程长阻塞（CHUNK=4096，从 O(n²) 次字符串拼接降为 ~n/4096 次）
function uint8ToBase64(uint8: Uint8Array): string {
  const CHUNK = 4096;
  const chunks: string[] = [];
  for (let i = 0; i < uint8.length; i += CHUNK) {
    chunks.push(String.fromCharCode(...uint8.subarray(i, i + CHUNK)));
  }
  return window.btoa(chunks.join(""));
}

function extOf(name: string): string {
  return name.split(".").pop()?.toLowerCase() ?? "";
}
function baseName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

// ===== 图片选择 =====
const allowedTypes = ["image/png", "image/jpeg", "image/gif", "image/webp"];
const maxImageSize = 5 * 1024 * 1024; // 5MB
const maxImageCount = 20;

async function pickImages() {
  const files = await open({
    multiple: true,
    filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "gif", "webp"] }],
  });
  if (!files) return;
  const paths = Array.isArray(files) ? files : [files];

  for (const filePath of paths) {
    if (chat.pendingImages.length >= maxImageCount) break;
    try {
      const uint8 = await readFile(filePath);
      const ext = extOf(filePath);
      const mediaType = ext === "png" ? "image/png" : ext === "jpg" || ext === "jpeg" ? "image/jpeg" : ext === "gif" ? "image/gif" : ext === "webp" ? "image/webp" : "image/png";
      if (!allowedTypes.includes(mediaType)) continue;
      if (uint8.length > maxImageSize) continue;
      chat.pendingImages.push({ data: uint8ToBase64(uint8), mediaType, name: baseName(filePath) });
    } catch (e) {
      console.error("读取图片失败:", e);
    }
  }
}

function removeImage(index: number) {
  chat.pendingImages.splice(index, 1);
}

// ===== office/pdf 文件附件 =====
// 后端在 send_message 入口 materialize 为 Text 块（doc::try_extract），前端只收集 base64 + 文件名。
// 扩展名白名单/上限与后端 infra::file_validation 对齐（docx/xlsx/xls/pdf，单文件 20MB，最多 10 个）。
const allowedFileExts = ["docx", "xlsx", "xls", "pdf"];
const maxFileSize = 20 * 1024 * 1024; // 20MB
const maxFileCount = 10;

/** 推入一个文件（校验扩展名/大小/数量），返回是否成功 */
function pushFile(name: string, bytes: Uint8Array): boolean {
  if (!allowedFileExts.includes(extOf(name))) return false;
  if (bytes.length > maxFileSize) return false;
  if (chat.pendingFiles.length >= maxFileCount) return false;
  chat.pendingFiles.push({ name, data: uint8ToBase64(bytes), size: bytes.length });
  return true;
}

/** 文件选择对话框（按钮触发） */
async function pickFiles() {
  const files = await open({
    multiple: true,
    filters: [{ name: "文档", extensions: allowedFileExts }],
  });
  if (!files) return;
  const paths = Array.isArray(files) ? files : [files];
  for (const filePath of paths) {
    if (chat.pendingFiles.length >= maxFileCount) break;
    try {
      const uint8 = await readFile(filePath);
      pushFile(baseName(filePath), uint8);
    } catch (e) {
      console.error("读取文件失败:", e);
    }
  }
}

/** 从浏览器 File 列表（拖拽/粘贴）批量加入附件 */
async function addFilesFromFileList(fileList: File[]): Promise<void> {
  for (const file of fileList) {
    if (chat.pendingFiles.length >= maxFileCount) break;
    try {
      const buf = new Uint8Array(await file.arrayBuffer());
      pushFile(file.name, buf);
    } catch (e) {
      console.error("读取文件失败:", e);
    }
  }
}

function removeFile(index: number) {
  chat.pendingFiles.splice(index, 1);
}

/** 字节数 → 人读尺寸 */
function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

// 拖拽高亮状态
const dragOver = ref(false);
function onDragOver(e: DragEvent) {
  if (!e.dataTransfer?.types?.includes("Files")) return;
  e.preventDefault();
  dragOver.value = true;
}
function onDragLeave(e: DragEvent) {
  // 仅当离开容器本身时清除（避免在子元素间进出抖动）
  if (e.currentTarget === e.target) dragOver.value = false;
}
async function onDrop(e: DragEvent) {
  const files = e.dataTransfer?.files;
  if (!files?.length) return;
  e.preventDefault();
  dragOver.value = false;
  await addFilesFromFileList(Array.from(files));
}
async function onPaste(e: ClipboardEvent) {
  const files = e.clipboardData?.files;
  if (!files?.length) return; // 纯文本粘贴放行默认行为
  e.preventDefault();
  await addFilesFromFileList(Array.from(files));
}

// ===== 发送 =====
function send() {
  const text = input.value.trim();
  if ((!text && chat.pendingImages.length === 0 && chat.pendingFiles.length === 0) || chat.sending) return;

  const blocks: ContentBlock[] = [];
  if (text) blocks.push({ type: "text", text });
  chat.draftText = "";
  chat.sendMessage(text, blocks);
  nextTick(() => {
    const el = textareaRef.value;
    if (el) el.style.height = "auto";
  });
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    send();
  }
  // U6: 防止 Backspace 在只读/禁用状态下触发浏览器后退
  if (e.key === "Backspace" && (e.target as HTMLTextAreaElement)?.disabled) {
    e.preventDefault();
  }
}
</script>

<template>
  <div class="input-area">
    <div class="input-container">
      <!-- 图片预览 -->
      <div v-if="chat.pendingImages.length > 0" class="preview-strip">
        <div v-for="(img, idx) in chat.pendingImages" :key="idx" class="preview-item">
          <img :src="`data:${img.mediaType};base64,${img.data}`" class="preview-thumb" />
          <button class="preview-remove" @click="removeImage(idx)">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
          </button>
        </div>
      </div>

      <!-- 文件附件预览 -->
      <div v-if="chat.pendingFiles.length > 0" class="file-strip">
        <div v-for="(f, idx) in chat.pendingFiles" :key="idx" class="file-chip">
          <svg class="file-chip-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" /></svg>
          <span class="file-chip-name" :title="f.name">{{ f.name }}</span>
          <span class="file-chip-size">{{ formatSize(f.size) }}</span>
          <button class="file-chip-remove" title="移除" @click="removeFile(idx)">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
          </button>
        </div>
      </div>

      <div class="input-wrapper" :class="{ 'is-sending': chat.sending, 'drag-over': dragOver }" @dragover="onDragOver" @dragleave="onDragLeave" @drop="onDrop">
        <div class="input-row">
          <textarea
            ref="textareaRef"
            v-model="input"
            class="chat-textarea"
            placeholder="输入消息…（可拖拽/粘贴 office、pdf 附件）"
            rows="1"
            :disabled="chat.sending"
            @keydown="handleKeydown"
            @input="autoResize"
            @paste="onPaste"
          />
          <div class="btn-group">
            <button v-if="!chat.sending" class="btn-send" :class="{ active: input.trim() || chat.pendingImages.length > 0 || chat.pendingFiles.length > 0 }" :disabled="!input.trim() && chat.pendingImages.length === 0 && chat.pendingFiles.length === 0" title="发送 (Enter)" @click="send">
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <line x1="22" y1="2" x2="11" y2="13" /><polygon points="22 2 15 22 11 13 2 9 22 2" />
              </svg>
            </button>
            <button v-else class="btn-stop" title="停止生成" @click="chat.stopGeneration()">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="2" /></svg>
            </button>
          </div>
        </div>
        <div class="input-footer">
          <button class="btn-img" :disabled="chat.sending" title="添加图片" @click="pickImages">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2" ry="2" /><circle cx="8.5" cy="8.5" r="1.5" /><polyline points="21 15 16 10 5 21" /></svg>
          </button>
          <button class="btn-img" :disabled="chat.sending" title="添加文档（docx / xlsx / xls / pdf）" @click="pickFiles">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" /><line x1="9" y1="13" x2="15" y2="13" /><line x1="9" y1="17" x2="15" y2="17" /></svg>
          </button>
        </div>
      </div>
      <p class="input-hint">{{ chat.sending ? "正在生成…" : "Enter 发送 · Shift+Enter 换行" }}</p>
    </div>
  </div>
</template>

<style scoped>
.input-area { flex-shrink:0; padding:16px 24px 24px; border-top:1px solid var(--color-chat-header-border); }
.input-container { max-width:800px; margin:0 auto; display:flex; flex-direction:column; gap:8px; }

/* ===== 图片预览条 ===== */
.preview-strip { display:flex; gap:8px; flex-wrap:wrap; }
.preview-item { position:relative; width:72px; height:72px; border-radius:var(--ip-radius-lg); overflow:hidden; border:1px solid var(--ip-color-border-default); }
.preview-thumb { width:100%; height:100%; object-fit:cover; }
.preview-remove { position:absolute; top:2px; right:2px; width:20px; height:20px; border-radius:50%; background:rgba(0,0,0,0.5); color:white; border:none; cursor:pointer; display:flex; align-items:center; justify-content:center; opacity:0; transition:opacity var(--ip-duration-fast) var(--ip-ease-out); }
.preview-item:hover .preview-remove { opacity:1; }

/* ===== 文件附件预览条 ===== */
.file-strip { display:flex; gap:6px; flex-wrap:wrap; }
.file-chip { display:flex; align-items:center; gap:6px; max-width:240px; padding:4px 8px; border-radius:var(--ip-radius-md); background-color:var(--ip-color-bg-tertiary); border:1px solid var(--ip-color-border-default); color:var(--ip-color-text-secondary); font-size:12px; }
.file-chip-icon { flex-shrink:0; color:var(--ip-primary-600); }
.file-chip-name { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.file-chip-size { flex-shrink:0; color:var(--ip-color-text-disabled); font-size:11px; }
.file-chip-remove { flex-shrink:0; display:flex; align-items:center; justify-content:center; width:16px; height:16px; border-radius:50%; border:none; background:transparent; color:var(--ip-color-text-disabled); cursor:pointer; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.file-chip-remove:hover { background-color:var(--ip-color-bg-hover); color:var(--ip-danger-base); }

.input-wrapper { display:flex; flex-direction:column; background-color:var(--color-input-bg); border:1px solid var(--color-input-border); border-radius:12px; transition:border-color var(--ip-duration-base) var(--ip-ease-out),box-shadow var(--ip-duration-base) var(--ip-ease-out); }
.input-row { display:flex; align-items:flex-start; gap:4px; padding:8px 8px 0 12px; }
.input-wrapper:focus-within { border-color:var(--color-input-focus-border); box-shadow:0 0 0 3px rgba(46,141,100,0.12); }
.input-wrapper.is-sending { border-color:var(--ip-primary-400); box-shadow:0 0 0 3px rgba(46,141,100,0.08); }
.input-wrapper.drag-over { border-color:var(--ip-primary-500); box-shadow:0 0 0 3px rgba(46,141,100,0.18); background-color:var(--ip-primary-50); }

.input-footer { display:flex; align-items:center; gap:2px; padding:0 4px 4px 4px; }
.btn-img { display:flex; align-items:center; justify-content:center; width:24px; height:24px; border-radius:var(--ip-radius-md); border:none; background:transparent; color:var(--ip-color-text-tertiary); cursor:pointer; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.btn-img:hover { background-color:var(--ip-color-bg-tertiary); color:var(--ip-primary-600); }
.btn-img:disabled { opacity:0.35; cursor:not-allowed; }

.chat-textarea { flex:1; border:none; outline:none; background:transparent; resize:none; font-size:var(--ip-text-body-size); line-height:1.5; color:var(--ip-color-text-primary); max-height:200px; min-height:22px; padding:4px 0 0; overflow-y:auto; }
.chat-textarea::placeholder { color:var(--ip-color-text-placeholder); }
.chat-textarea:disabled { opacity:0.35; cursor:not-allowed; }

.btn-group { position:relative; width:36px; height:36px; flex-shrink:0; }
.btn-send { position:absolute; inset:0; display:flex; align-items:center; justify-content:center; border-radius:var(--ip-radius-md); background-color:var(--ip-color-bg-tertiary); color:var(--ip-color-text-disabled); border:none; cursor:pointer; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.btn-send.active { background-color:var(--color-message-user-bg); color:white; }
.btn-send.active:hover { opacity:0.9; transform:scale(1.05); }
.btn-send.active:active { transform:scale(0.95); }
.btn-stop { position:absolute; inset:0; display:flex; align-items:center; justify-content:center; border-radius:var(--ip-radius-md); background-color:var(--ip-danger-base); color:white; border:none; cursor:pointer; transition:all var(--ip-duration-fast) var(--ip-ease-out); animation:stop-enter 0.2s ease-out; }
.btn-stop:hover { opacity:0.9; }
@keyframes stop-enter { from { opacity:0; transform:scale(0.85); } to { opacity:1; transform:scale(1); } }
.input-hint { font-size:11px; color:var(--ip-color-text-disabled); text-align:center; }
</style>
