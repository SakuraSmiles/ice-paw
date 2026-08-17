<!--
  ChatInput — 聊天输入框 + 图片/文档附件 + 发送/停止按钮

  行为：
  - Enter 发送，Shift+Enter 换行
  - 附件（图片 / docx / xlsx / xls / pdf）：**一个按钮**选择 / 拖拽 / 粘贴，按扩展名分流
    → 图片走 base64 ContentBlock（image）；文档走后端 materialize 为 Text 块
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
import { useAgentStore } from "../../stores/agent";
import { shortCode } from "../../utils/refs";
import type { ContentBlock } from "../../types";

const chat = useChatStore();
const agentStore = useAgentStore();
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

/** textarea 原生 input：自适应高度 + @ 引用触发检测。*/
function onInput() {
  autoResize();
  detectAtTrigger();
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

// ===== 附件类型常量（图片 + 文档统一管理）=====
// 图片：base64 ContentBlock（image），前端直传；文档：后端 materialize 为 Text（doc::try_extract）。
const imageExts = ["png", "jpg", "jpeg", "gif", "webp"];
const docExts = ["docx", "xlsx", "xls", "pdf"]; // 与后端 infra::file_validation 对齐
const allowedTypes = ["image/png", "image/jpeg", "image/gif", "image/webp"];
const maxImageSize = 5 * 1024 * 1024; // 5MB
const maxImageCount = 20;
// 100MB：后端 materialize 对大文档分页（首页内联 + read_attachment_page），LLM 窗口非瓶颈；
// 此上限是前端 base64 编码内存折中（100MB≈133MB 字符串）。须与后端 file_validation MAX_FILE_SIZE 同步。
const maxFileSize = 100 * 1024 * 1024; // 100MB
const maxFileCount = 10;

/** 图片扩展名 → MIME（与 allowedTypes 对齐）*/
function extToMediaType(ext: string): string {
  switch (ext) {
    case "png": return "image/png";
    case "jpg": case "jpeg": return "image/jpeg";
    case "gif": return "image/gif";
    case "webp": return "image/webp";
    default: return "image/png";
  }
}

/** 附件被拒原因（pushAttachment 返回；true = 入队成功）。*/
type AttachReject = "empty" | "tooLarge" | "unsupported" | "tooMany";
type PushResult = true | AttachReject;

/** 拒绝原因 → 人读文案（聚合展示用）。*/
const REJECT_LABEL: Record<AttachReject, string> = {
  empty: "为空（0 字节）",
  tooLarge: "超过大小上限",
  unsupported: "格式不支持",
  tooMany: "超过数量上限",
};

/**
 * 推入一个已读字节流：图片 → pendingImages，文档 → pendingFiles。
 * 返回 `true` 表示入队成功，否则返回拒绝原因（0 字节 / 超大 / 格式不支持 / 超数量上限）。
 *
 * **0 字节拦截**（用户痛点）：0 字节文档会让后端提取失败进而拖死整条消息、0 字节图片
 * 会被 LLM 以 400 拒绝。在前端入口直接拦截，坏附件根本不进待发列表；后端另有软失败兜底。
 */
function pushAttachment(name: string, bytes: Uint8Array): PushResult {
  if (bytes.length === 0) return "empty";
  const ext = extOf(name);
  if (imageExts.includes(ext)) {
    const mediaType = extToMediaType(ext);
    if (!allowedTypes.includes(mediaType)) return "unsupported";
    if (bytes.length > maxImageSize) return "tooLarge";
    if (chat.pendingImages.length >= maxImageCount) return "tooMany";
    chat.pendingImages.push({ data: uint8ToBase64(bytes), mediaType, name });
    return true;
  }
  if (docExts.includes(ext)) {
    if (bytes.length > maxFileSize) return "tooLarge";
    if (chat.pendingFiles.length >= maxFileCount) return "tooMany";
    chat.pendingFiles.push({ name, data: uint8ToBase64(bytes), size: bytes.length });
    return true;
  }
  return "unsupported"; // 非图片/文档扩展名
}

/** 统一附件选择对话框（按钮触发）：图片 + 文档一个入口，按扩展名分流。*/
async function pickAttachments() {
  const files = await open({
    multiple: true,
    // 默认即"全部"（图片 + office 全范围），不细分图片/文档
    filters: [{ name: "全部", extensions: [...imageExts, ...docExts] }],
  });
  if (!files) return;
  const paths = Array.isArray(files) ? files : [files];
  const fails: { name: string; reason: string }[] = [];
  for (const filePath of paths) {
    const name = baseName(filePath);
    try {
      const uint8 = await readFile(filePath);
      const r = pushAttachment(name, uint8);
      if (r !== true) fails.push({ name, reason: REJECT_LABEL[r] });
    } catch (e) {
      console.error("读取附件失败:", e);
      fails.push({ name, reason: "读取失败" });
    }
  }
  reportAttachFails(fails);
}

/** 从浏览器 File 列表（拖拽/粘贴）批量加入附件，按扩展名分流图片/文档。*/
async function addAttachmentsFromFileList(fileList: File[]): Promise<void> {
  const fails: { name: string; reason: string }[] = [];
  for (const file of fileList) {
    try {
      const buf = new Uint8Array(await file.arrayBuffer());
      const r = pushAttachment(file.name, buf);
      if (r !== true) fails.push({ name: file.name, reason: REJECT_LABEL[r] });
    } catch (e) {
      console.error("读取附件失败:", e);
      fails.push({ name: file.name, reason: "读取失败" });
    }
  }
  reportAttachFails(fails);
}

function removeImage(index: number) {
  chat.pendingImages.splice(index, 1);
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

// ===== 附件拒绝反馈（无全局 toast；内联警告条）=====
// pushAttachment 的失败（0 字节 / 超大 / 格式不支持 / 超数量上限 / 读取失败）聚合一条警告，
// 6 秒后自动清除。逐个弹 toast 噪声大；一条汇总更克制。
const attachWarn = ref("");
let attachWarnTimer: ReturnType<typeof setTimeout> | null = null;
function reportAttachFails(fails: { name: string; reason: string }[]) {
  if (!fails.length) return;
  const lines = fails.map((f) => `• ${f.name}：${f.reason}`).join("\n");
  attachWarn.value = `以下附件未添加：\n${lines}`;
  if (attachWarnTimer) clearTimeout(attachWarnTimer);
  attachWarnTimer = setTimeout(() => {
    attachWarn.value = "";
    attachWarnTimer = null;
  }, 6000);
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
  await addAttachmentsFromFileList(Array.from(files));
}
async function onPaste(e: ClipboardEvent) {
  const files = e.clipboardData?.files;
  if (!files?.length) return; // 纯文本粘贴放行默认行为
  e.preventDefault();
  await addAttachmentsFromFileList(Array.from(files));
}

// ===== @ 引用（会话 / agent / 消息）=====
// 触发：光标前最近的 `@`（其前是行首/空白/起点，其后到光标无空白）进入引用
// 模式，`@` 后文本作过滤 query。选中后删掉 `@query` 文本、push 引用 chip——
// 输入框保持纯文本，引用以 chip 徽章形态存在（可删，非裸文本）。
const atQuery = ref<string | null>(null); // null = 未激活

function detectAtTrigger() {
  const el = textareaRef.value;
  if (!el) return;
  const pos = el.selectionStart ?? 0;
  const before = el.value.slice(0, pos);
  const atIdx = before.lastIndexOf("@");
  if (atIdx === -1 || atQueryClosed(before, atIdx)) {
    atQuery.value = null;
    activeIdx.value = 0;
    return;
  }
  atQuery.value = before.slice(atIdx + 1);
}

/** @ 是否失效：前一个字符非空白（如邮箱 a@b），或 @ 到光标之间出现空白（词已结束）。*/
function atQueryClosed(before: string, atIdx: number): boolean {
  if (atIdx > 0 && !/\s/.test(before[atIdx - 1])) return true;
  return /\s/.test(before.slice(atIdx + 1));
}

/** 弹层候选（三段：会话 / Agent / 当前会话消息）。*/
interface RefOption {
  kind: "conversation" | "agent" | "message";
  targetId: string;
  display: string; // `名称#短码`（落库展示，后端失效降级也用它）
  label: string; // 弹层主行
  sub: string; // 弹层副行（kind 徽标）
}

/** 会话标题：空 → 「会话」+ 短码（未命名会话显示 `会话#3357` 兜底）。*/
function convLabel(title: string): string {
  return title.trim() || "会话";
}

const SECTION_CAP = 5; // 每段最多展示条数（克制）
const MESSAGE_POOL = 20; // 消息段参与过滤的池（当前会话最近 N 条）

const refOptions = computed<RefOption[]>(() => {
  if (atQuery.value === null) return [];
  const q = atQuery.value.toLowerCase();
  const match = (s: string) => !q || s.toLowerCase().includes(q);
  const out: RefOption[] = [];

  // 会话：排除委派子会话（侧栏不可见）与当前会话（其历史本就在上下文里）
  for (const c of chat.conversations) {
    if (c.kind === "delegation" || c.id === chat.activeConvId) continue;
    const label = convLabel(c.title);
    if (!match(label) && !match(c.title)) continue;
    const code = shortCode(c.id);
    out.push({
      kind: "conversation", targetId: c.id,
      display: `${label}#${code}`, label, sub: "会话",
    });
    if (out.length >= SECTION_CAP) break;
  }
  const convCount = out.length;

  // Agent：身份卡语义（name + desc），与 delegate 权限零冲突
  for (const a of agentStore.list) {
    if (!match(a.name)) continue;
    const code = shortCode(a.id);
    out.push({
      kind: "agent", targetId: a.id,
      display: `${a.name}#${code}`, label: a.name, sub: `Agent · ${a.model}`,
    });
    if (out.length - convCount >= SECTION_CAP) break;
  }
  const agentCount = out.length - convCount;

  // 消息：当前会话最近 N 条（倒序取池再正序展示），占位行（tool_result / 空内容）不参与
  const pool = chat.messages
    .filter((m) => m.content.trim() && (m.role === "user" || m.role === "assistant"))
    .slice(-MESSAGE_POOL)
    .reverse();
  for (const m of pool) {
    if (!match(m.content)) continue;
    const code = shortCode(m.id);
    const kindLabel = m.role === "assistant" ? "回答" : "消息";
    out.push({
      kind: "message", targetId: m.id,
      display: `${kindLabel}#${code}`,
      label: m.content.length > 60 ? m.content.slice(0, 60) + "…" : m.content,
      sub: kindLabel,
    });
    if (out.length - convCount - agentCount >= SECTION_CAP) break;
  }
  return out;
});

const atActive = computed(() => atQuery.value !== null && refOptions.value.length > 0);
const activeIdx = ref(0);
watch(refOptions, () => { activeIdx.value = 0; });

/** 选中候选：删掉 `@query` 文本 + push chip + 关弹层 + 焦点回输入框。*/
function chooseRef(opt: RefOption) {
  const el = textareaRef.value;
  if (el) {
    const pos = el.selectionStart ?? 0;
    const atIdx = el.value.slice(0, pos).lastIndexOf("@");
    if (atIdx >= 0) {
      const next = el.value.slice(0, atIdx) + el.value.slice(pos);
      input.value = next;
      nextTick(() => el.setSelectionRange(atIdx, atIdx));
    }
  }
  chat.pendingRefs.push({ refKind: opt.kind, targetId: opt.targetId, display: opt.display });
  atQuery.value = null;
  activeIdx.value = 0;
  nextTick(() => textareaRef.value?.focus());
}

function removeRef(index: number) {
  chat.pendingRefs.splice(index, 1);
}

/** @ 按钮：光标处插入 @ 并触发弹层（与手输 @ 同效果，提升可发现性）。
 *  光标前非空白且非起点时补一个空格，保证触发条件（@ 前行首/空白）成立。*/
async function insertAtTrigger() {
  const el = textareaRef.value;
  if (!el || chat.sending) return;
  const pos = el.selectionStart ?? el.value.length;
  const prev = el.value.slice(0, pos);
  const insert = pos > 0 && !/\s/.test(prev[prev.length - 1]) ? " @" : "@";
  input.value = prev + insert + el.value.slice(pos);
  await nextTick(); // v-model 同步到 DOM 后再定位光标
  const newPos = pos + insert.length;
  el.focus();
  el.setSelectionRange(newPos, newPos);
  autoResize();
  detectAtTrigger(); // 程序化改值不触发 @input，手动检测
}

// ===== 发送 =====
function send() {
  const text = input.value.trim();
  if ((!text && chat.pendingImages.length === 0 && chat.pendingFiles.length === 0 && chat.pendingRefs.length === 0) || chat.sending) return;

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
  // @ 弹层键盘导航（激活时优先于发送）
  if (atActive.value) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      activeIdx.value = (activeIdx.value + 1) % refOptions.value.length;
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      activeIdx.value = (activeIdx.value - 1 + refOptions.value.length) % refOptions.value.length;
      return;
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      chooseRef(refOptions.value[activeIdx.value]);
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      atQuery.value = null;
      return;
    }
  }
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

      <!-- @ 引用 chip 条（复用 file-chip 视觉） -->
      <div v-if="chat.pendingRefs.length > 0" class="file-strip">
        <div v-for="(r, idx) in chat.pendingRefs" :key="r.refKind + ':' + r.targetId" class="file-chip ref-chip" :data-ref-kind="r.refKind">
          <svg class="file-chip-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" /><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" /></svg>
          <span class="file-chip-name" :title="r.display">{{ r.display }}</span>
          <button class="file-chip-remove" title="移除" @click="removeRef(idx)">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
          </button>
        </div>
      </div>

      <div class="input-wrapper" :class="{ 'is-sending': chat.sending, 'drag-over': dragOver }" @dragover="onDragOver" @dragleave="onDragLeave" @drop="onDrop">
        <!-- @ 引用弹层（输入框上方；mousedown.prevent 保证点选不丢 textarea 焦点） -->
        <div v-if="atActive" class="at-popover">
          <button
            v-for="(opt, i) in refOptions"
            :key="opt.kind + ':' + opt.targetId"
            class="at-option"
            :class="{ active: i === activeIdx }"
            type="button"
            @mousedown.prevent
            @click="chooseRef(opt)"
            @mousemove="activeIdx = i"
          >
            <span class="at-option-label" :title="opt.label">{{ opt.label }}</span>
            <span class="at-option-sub">{{ opt.sub }}</span>
          </button>
        </div>
        <div class="input-row">
          <textarea
            ref="textareaRef"
            v-model="input"
            class="chat-textarea"
            placeholder="输入消息…（可拖拽/粘贴附件）"
            rows="1"
            :disabled="chat.sending"
            @keydown="handleKeydown"
            @input="onInput"
            @paste="onPaste"
            @blur="atQuery = null"
          />
          <div class="btn-group">
            <button v-if="!chat.sending" class="btn-send" :class="{ active: input.trim() || chat.pendingImages.length > 0 || chat.pendingFiles.length > 0 || chat.pendingRefs.length > 0 }" :disabled="!input.trim() && chat.pendingImages.length === 0 && chat.pendingFiles.length === 0 && chat.pendingRefs.length === 0" title="发送 (Enter)" @click="send">
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
          <button class="btn-img" :disabled="chat.sending" title="添加附件（图片 / docx / xlsx / xls / pdf）" @click="pickAttachments">
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" /></svg>
          </button>
          <button class="btn-img btn-at" :disabled="chat.sending" title="引用（@ 会话 / Agent / 消息）" @click="insertAtTrigger">
            <span class="at-glyph">@</span>
          </button>
        </div>
      </div>
      <p v-if="attachWarn" class="attach-warn">{{ attachWarn }}</p>
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

.input-wrapper { position:relative; display:flex; flex-direction:column; background-color:var(--color-input-bg); border:1px solid var(--color-input-border); border-radius:12px; transition:border-color var(--ip-duration-base) var(--ip-ease-out),box-shadow var(--ip-duration-base) var(--ip-ease-out); }

/* ===== @ 引用弹层（输入框上方） ===== */
.at-popover { position:absolute; left:8px; right:8px; bottom:calc(100% + 6px); z-index:20; max-height:280px; overflow-y:auto; background-color:var(--ip-color-bg-primary); border:1px solid var(--ip-color-border-default); border-radius:var(--ip-radius-md); box-shadow:0 4px 16px rgba(0,0,0,0.12); padding:4px; }
.at-option { display:flex; align-items:center; gap:8px; width:100%; padding:7px 10px; border:none; border-radius:var(--ip-radius-sm); background:transparent; cursor:pointer; text-align:left; }
.at-option.active { background-color:var(--ip-color-bg-hover); }
.at-option-label { flex:1; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-size:13px; color:var(--ip-color-text-primary); }
.at-option-sub { flex-shrink:0; font-size:11px; color:var(--ip-color-text-disabled); }

/* @ 引用 chip：按 ref_kind 微调图标色（会话=主色 / agent=紫 / 消息=中性） */
.ref-chip[data-ref-kind="agent"] .file-chip-icon { color:#7c6bd6; }
.ref-chip[data-ref-kind="message"] .file-chip-icon { color:var(--ip-color-text-secondary); }
.input-row { display:flex; align-items:flex-start; gap:4px; padding:8px 8px 0 12px; }
.input-wrapper:focus-within { border-color:var(--color-input-focus-border); box-shadow:0 0 0 3px rgba(46,141,100,0.12); }
.input-wrapper.is-sending { border-color:var(--ip-primary-400); box-shadow:0 0 0 3px rgba(46,141,100,0.08); }
.input-wrapper.drag-over { border-color:var(--ip-primary-500); box-shadow:0 0 0 3px rgba(46,141,100,0.18); background-color:var(--ip-primary-50); }

.input-footer { display:flex; align-items:center; gap:2px; padding:0 4px 4px 4px; }
.btn-img { display:flex; align-items:center; justify-content:center; width:24px; height:24px; border-radius:var(--ip-radius-md); border:none; background:transparent; color:var(--ip-color-text-tertiary); cursor:pointer; transition:all var(--ip-duration-fast) var(--ip-ease-out); }
.btn-img:hover { background-color:var(--ip-color-bg-tertiary); color:var(--ip-primary-600); }
.btn-img:disabled { opacity:0.35; cursor:not-allowed; }
/* @ 引用按钮：字符图标（加粗微调基线，视觉与 16px 线性图标等重） */
.btn-at .at-glyph { font-size:14px; font-weight:700; line-height:1; }

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
.attach-warn { font-size:12px; line-height:1.5; white-space:pre-line; color:var(--ip-danger-base); text-align:center; margin:0; }
</style>
