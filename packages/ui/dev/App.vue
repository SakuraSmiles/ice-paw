<script setup lang="ts">
/**
 * IcePaw UI · 预览站 v2
 *
 * 类型: 品牌展示落地页（取代 v1 的组件文档目录）
 * 规范: wave2-preview-site-spec.md（ui-designer 出品）
 * 视觉基准: previews/ui-hero-v2.html
 *
 * 实现要点:
 * - Hero 双栏 grid（左文案 + 右聊天预览 + 4 个浮动卡 + 冰晶 SVG 背景）
 * - 13 个 section，每个 section 布局不同（无 5 段式模板）
 * - 14 个组件 demo 数据全部保留
 * - 字体三层: display (Instrument Serif) / sans (DM Sans) / mono (JetBrains Mono)
 * - 暗色模式: [data-theme="dark"]
 * - 响应式: <1024px / <768px 断点
 */
import { ref, onMounted, onUnmounted, h } from 'vue'
import {
  Button,
  Input,
  Textarea,
  MessageBubble,
  Modal,
  ToastContainer,
  provideToast,
  IpFlex,
  IpContainer,
  IpAvatar,
  IpCard,
  IpSelect,
  IpEmptyState,
  IpDropdownMenu,
  IpPopconfirm,
  type ToastApi,
} from '../src'
import {
  Search,
  AtSign,
  Hash,
  RotateCcw,
  Settings,
  MoreHorizontal,
  Plus,
  Send,
  Trash2,
  Bot,
  Inbox,
  Sparkles,
  Pencil,
  Copy,
  Download,
  ChevronDown,
  CheckCircle,
  AlertCircle,
  LogOut,
  Share2,
  Sun,
  Moon,
} from 'lucide-vue-next'

/* ── Theme ── */
const theme = ref<'light' | 'dark'>('light')
function toggleTheme(): void {
  theme.value = theme.value === 'light' ? 'dark' : 'light'
  document.documentElement.setAttribute('data-theme', theme.value)
}

/* ── Toast API ── */
const toast: ToastApi = provideToast()

/* ── Demo state (preserved from v1) ── */
const inputValue = ref<string>('你好')
const inputError = ref<boolean>(false)
const inputErrorMessage = ref<string>('')

const taValue = ref<string>(
  '今天天气不错，适合写代码。\n不过下午可能要开会。',
)

const modalOpenSm = ref(false)
const modalOpenMd = ref(false)
const modalOpenLg = ref(false)

const btnLoading = ref(false)
function triggerLoading(): void {
  btnLoading.value = true
  setTimeout(() => { btnLoading.value = false }, 1500)
}

let inputErrorTimer: ReturnType<typeof setTimeout> | undefined
function showInputError(): void {
  if (inputErrorTimer) clearTimeout(inputErrorTimer)
  inputError.value = false
  inputErrorMessage.value = ''
  setTimeout(() => {
    inputError.value = true
    inputErrorMessage.value = '用户名已被占用'
    inputErrorTimer = setTimeout(() => {
      inputError.value = false
      inputErrorMessage.value = ''
      inputErrorTimer = undefined
    }, 3000)
  }, 50)
}

function clearInputError(): void {
  inputError.value = false
  inputErrorMessage.value = ''
}

const longMessage = ref<string>(
  '推荐使用 `fetch` + `AbortController` 实现可中断的请求。' +
  '配合 `useRef` 持有 controller 实例，在 cleanup 里调用 `abort()` 即可取消挂起的请求。',
)

function toastSuccess(): void { toast.success('保存成功') }
function toastError(): void { toast.error('保存失败') }
function toastWarning(): void { toast.warning('网络连接不稳定') }
function toastInfo(): void { toast.info('检测到新版本') }
function toastMergeDemo(): void {
  toast.success('第一次保存', { title: '保存中' })
  setTimeout(() => toast.success('第二次保存'), 800)
  setTimeout(() => toast.success('第三次保存'), 1600)
}

/* ── Nav active 联动 ── */
const navItems = [
  { label: '概览', href: '#hero', id: 'hero' },
  { label: '组件', href: '#components', id: 'components' },
] as const
const activeNav = ref<typeof navItems[number]['label']>('概览')

let cleanupActiveObserver: (() => void) | undefined
function initActiveObserver(): () => void {
  const observer = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (entry.isIntersecting && entry.target.id) {
          const match = navItems.find((n) => n.id === entry.target.id)
          if (match) activeNav.value = match.label
        }
      })
    },
    { threshold: 0.3 },
  )
  navItems.forEach((n) => {
    const el = document.getElementById(n.id)
    if (el) observer.observe(el)
  })
  return () => observer.disconnect()
}

/* ── Section 1 Buttons demo state ── */
const stateView = ref<'default' | 'hover' | 'active' | 'disabled' | 'loading'>('default')
const btnVariants = [
  { key: 'primary',   label: 'Primary',   sub: '主操作' },
  { key: 'secondary', label: 'Secondary', sub: '次要操作' },
  { key: 'ghost',     label: 'Ghost',     sub: '弱操作' },
  { key: 'danger',    label: 'Danger',    sub: '危险操作' },
] as const

/* ── Section 4 Inputs demo state ── */
const auxValue = ref<string>('你好')
const agentName = ref<string>('代码伙伴')
const agentPrompt = ref<string>('你是一个乐于助人的代码伙伴。')

/* ── Section 7 Avatar demo state ── */
const avatarSrc = ref<string>(
  'https://api.dicebear.com/9.x/notionists/svg?seed=icepaw&backgroundColor=d1d4f9',
)
const defaultAvatar = avatarSrc.value
const avatarCleared = ref(false)

function clearAvatar(): void {
  avatarSrc.value = ''
  avatarCleared.value = true
  toast.info('已清除头像')
  setTimeout(() => {
    avatarSrc.value = defaultAvatar
    avatarCleared.value = false
  }, 1000)
}
function onAvatarUpload(file: File): void {
  const reader = new FileReader()
  reader.onload = (ev) => {
    avatarSrc.value = String(ev.target?.result ?? '')
    toast.success(`已上传 ${file.name}`)
  }
  reader.readAsDataURL(file)
}
function onAvatarError(err: { code: string; message: string }): void {
  toast.error(err.message)
}

/* ── Section 2 Cards demo state ── */
const cardSelected = ref(false)

/* ── Section 8 Select demo state ── */
const selectValue = ref<string | null>('balanced')
const selectError = ref(false)
const selectErrorMessage = ref('')
const selectClearable = ref<string | null>('gemini-2.5-pro')
const selectTone = ref<string | null>('balanced')

const modelOptions = [
  { value: 'gpt-4o', label: 'GPT-4o', description: 'OpenAI · 多模态旗舰', icon: Sparkles },
  { value: 'claude-sonnet-4', label: 'Claude Sonnet 4', description: 'Anthropic · 长上下文', icon: Bot },
  { value: 'gemini-2.5-pro', label: 'Gemini 2.5 Pro', description: 'Google · 超大上下文窗口', icon: Sparkles },
  { value: 'deepseek-v3', label: 'DeepSeek V3', description: '深度求索 · 高性价比', icon: Sparkles },
  { value: 'llama-local', label: 'Llama (本地)', description: '离线可用', icon: Bot, disabled: true },
]

const toneOptions = [
  { value: 'concise', label: '简洁' },
  { value: 'balanced', label: '均衡' },
  { value: 'detailed', label: '详尽' },
  { value: 'creative', label: '创意' },
]

function triggerSelectError(): void {
  selectError.value = !selectError.value
  selectErrorMessage.value = selectError.value ? '请选择一个模型' : ''
}

/* ── Section 10 Dropdown demo state ── */
const ddOpen = ref(false)
const ddOpenHover = ref(false)
const ddOpenFull = ref(false)
const ddOpenDisabled = ref(false)
const ddAction = (label: string): void => {
  toast.info(`${label}`)
}

/* ── Section 11 Popconfirm demo state ── */
const popOpen = ref(false)
const popDangerOpen = ref(false)
const popLoading = ref(false)
const popTop = ref(false)
const popBottom = ref(false)
const popLeft = ref(false)
const popRight = ref(false)
function confirmDelete(): void {
  popLoading.value = true
  setTimeout(() => {
    popLoading.value = false
    popDangerOpen.value = false
    toast.success('已删除')
  }, 1200)
}

/* ── Dropdown demo data ── */
const ddItems = [
  { type: 'item' as const, key: 'edit', label: '编辑', icon: Pencil, onClick: () => ddAction('编辑') },
  { type: 'item' as const, key: 'duplicate', label: '复制', icon: Copy, onClick: () => ddAction('复制') },
  { type: 'item' as const, key: 'share', label: '分享', icon: Share2, onClick: () => ddAction('分享') },
  { type: 'divider' as const, key: 'div1' },
  { type: 'item' as const, key: 'download', label: '导出', icon: Download, onClick: () => ddAction('导出') },
  { type: 'item' as const, key: 'delete', label: '删除', icon: Trash2, danger: true, onClick: () => ddAction('删除') },
]

const ddFullItems = [
  { type: 'label' as const, text: '操作', key: 'ops-label' },
  { type: 'item' as const, key: 'edit2', label: '编辑', icon: Pencil, shortcut: 'E', onClick: () => ddAction('编辑') },
  { type: 'item' as const, key: 'duplicate2', label: '复制', icon: Copy, shortcut: 'D', onClick: () => ddAction('复制') },
  { type: 'item' as const, key: 'share2', label: '分享', icon: Share2, shortcut: 'S', onClick: () => ddAction('分享') },
  { type: 'divider' as const, key: 'div2' },
  { type: 'item' as const, key: 'download2', label: '导出', icon: Download, onClick: () => ddAction('导出') },
  { type: 'divider' as const, key: 'div3' },
  { type: 'item' as const, key: 'logout', label: '退出登录', icon: LogOut, danger: true, onClick: () => ddAction('退出登录') },
]

const ddDisabledItems = [
  { type: 'item' as const, key: 'view', label: '查看', icon: Search, onClick: () => ddAction('查看') },
  { type: 'item' as const, key: 'edit-disabled', label: '编辑', icon: Pencil, disabled: true },
  { type: 'item' as const, key: 'delete-disabled', label: '删除', icon: Trash2, danger: true, disabled: true },
  { type: 'divider' as const, key: 'div-disabled' },
  { type: 'item' as const, key: 'export', label: '导出', icon: Download, onClick: () => ddAction('导出') },
]

/* ── VNode separator demo ── */
const hCaretVNode = h(
  'svg',
  { width: 10, height: 10, viewBox: '0 0 10 10', 'aria-hidden': true },
  h('path', {
    d: 'M3 1 L7 5 L3 9',
    fill: 'none',
    stroke: 'currentColor',
    'stroke-width': '1.5',
    'stroke-linecap': 'round',
    'stroke-linejoin': 'round',
  }),
)

/* ── Section reveal (single fade-in, no stagger) ── */
function initSectionReveal(): () => void {
  const observer = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (entry.isIntersecting) {
          entry.target.classList.add('is-revealed')
          observer.unobserve(entry.target)
        }
      })
    },
    { threshold: 0.15, rootMargin: '0px 0px -10% 0px' },
  )
  const targets = document.querySelectorAll(
    '.section-divider, .buttons-section, .cards-section, .message-section, ' +
    '.inputs-section, .modals-section, .toast-section, .avatar-section, ' +
    '.select-section, .empty-section, .dropdown-section, .popconfirm-section, ' +
    '.flex-section, .container-section',
  )
  targets.forEach((el) => {
    el.classList.add('reveal-init')
    observer.observe(el)
  })
  return () => observer.disconnect()
}

let cleanupReveal: (() => void) | undefined

onMounted(() => {
  cleanupReveal = initSectionReveal()
  cleanupActiveObserver = initActiveObserver()
})

onUnmounted(() => {
  if (cleanupReveal) cleanupReveal()
  cleanupActiveObserver?.()
})
</script>

<template>
  <div class="preview-root" :data-theme="theme">
    <!-- Toast 容器 -->
    <ToastContainer />

    <!-- ══════════════ 顶部导航 ══════════════ -->
    <nav class="nav" aria-label="主导航">
      <div class="nav-inner">
        <a class="brand" href="#hero">
          <span class="brand-mark" aria-hidden="true">
            <svg viewBox="0 0 32 32" width="26" height="26">
              <rect width="32" height="32" rx="7" fill="var(--ip-primary-50)"/>
              <g transform="translate(5, 5)" fill="var(--ip-primary-500)">
                <ellipse cx="6"  cy="6"  rx="1.7" ry="2.3" transform="rotate(-25 6 6)"/>
                <ellipse cx="11" cy="2.8" rx="1.7" ry="2.3"/>
                <ellipse cx="16" cy="6"  rx="1.7" ry="2.3" transform="rotate(25 16 6)"/>
                <path d="M 4.5 12.5 Q 4.5 9.8, 7.8 9.3 L 14.2 9.3 Q 17.5 9.8, 17.5 12.5 Q 17.5 17.2, 11 18.2 Q 4.5 17.2, 4.5 12.5 Z"/>
              </g>
            </svg>
          </span>
          IcePaw<span class="accent"> · Design System</span>
        </a>

        <div class="nav-links">
          <a
            v-for="link in navItems"
            :key="link.label"
            class="nav-link"
            :class="{ active: activeNav === link.label }"
            :href="link.href"
          >{{ link.label }}</a>
        </div>

        <div class="nav-right">
          <a class="nav-link-quiet" href="#" aria-label="GitHub">GitHub</a>
          <button class="btn-sm primary" type="button">查看组件库</button>
          <button
            class="theme-toggle"
            type="button"
            :aria-label="theme === 'light' ? '切换到暗色模式' : '切换到亮色模式'"
            @click="toggleTheme"
          >
            <Sun v-if="theme === 'light'" :size="16" :stroke-width="2" aria-hidden="true" />
            <Moon v-else :size="16" :stroke-width="2" aria-hidden="true" />
          </button>
        </div>
      </div>
    </nav>

    <!-- ══════════════ HERO ══════════════ -->
    <section class="page hero" id="hero">
      <div class="hero-text">
        <div class="hero-eyebrow">
          <span class="pulse">v2</span>
          冰蓝品牌系统 · 2026 春季更新
        </div>

        <h1>
          为多 Agent<br/>
          项目打造的<em>对话界面</em>。
        </h1>

        <p class="hero-sub">
          14 个精心打磨的 Vue 3 组件 · 一套极地冰蓝的色彩体系 ·
          从消息气泡到模态框，每一处都兼顾克制与温度。
        </p>

        <div class="hero-cta">
          <a class="btn-lg primary" href="#components">
            浏览组件
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              <path d="M3 7h8m0 0L7.5 3.5M11 7l-3.5 3.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
          </a>
          <a class="btn-lg ghost" href="#components">查看组件</a>
        </div>

        <div class="hero-meta">
          <a href="#">npm i @icepaw/ui</a>
          <span class="dot"></span>
          <span>Vue 3.4+ · TypeScript · Tree-shakable</span>
          <span class="dot"></span>
          <a href="#">下载 v2 设计稿 (Figma)</a>
        </div>
      </div>

      <div class="hero-visual">
        <!-- 背景冰晶 SVG -->
        <div class="crystal-bg" aria-hidden="true">
          <svg viewBox="0 0 600 480" fill="none">
            <g stroke="var(--ip-primary-500)" stroke-width="1" stroke-linecap="round" opacity="0.6">
              <path d="M300 40 L300 440"/>
              <path d="M100 240 L500 240"/>
              <path d="M160 100 L440 380"/>
              <path d="M440 100 L160 380"/>
              <path d="M220 60 L380 60 M220 420 L380 420"/>
              <path d="M60 220 L60 260 M540 220 L540 260"/>
            </g>
            <g stroke="var(--ip-primary-500)" stroke-width="0.6" opacity="0.4">
              <path d="M300 100 L260 140 L260 180 L300 220"/>
              <path d="M300 100 L340 140 L340 180 L300 220"/>
              <path d="M220 240 L180 280 M380 240 L420 280"/>
              <path d="M260 240 L240 200 M340 240 L360 200"/>
            </g>
            <circle cx="300" cy="240" r="6" fill="var(--ip-primary-500)" opacity="0.5"/>
          </svg>
        </div>

        <!-- 浮动卡 1: Avatar -->
        <div class="float-card f1">
          <div class="label">Avatar</div>
          <div class="fc-row">
            <IpAvatar size="xs" :source="{ type: 'initials', text: '主', bgColor: 'var(--ip-primary-500)' }" />
            <IpAvatar size="xs" :source="{ type: 'initials', text: '研', bgColor: 'var(--ip-success-base)' }" />
            <IpAvatar size="xs" :source="{ type: 'initials', text: '审', bgColor: 'var(--ip-warning-base)' }" />
          </div>
        </div>

        <!-- 浮动卡 2: Badge -->
        <div class="float-card f2">
          <div class="label">Badge</div>
          <span class="mini-pill"><span class="dot"></span>已验证</span>
        </div>

        <!-- 浮动卡 3: Empty -->
        <div class="float-card f3">
          <div class="label">Empty</div>
          <div class="fc-text">项目里还没有 Agent</div>
        </div>

        <!-- 浮动卡 4: Toast -->
        <div class="float-card f4">
          <div class="label">Toast</div>
          <div class="fc-row">
            <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
              <circle cx="6" cy="6" r="5" stroke="var(--ip-success-base)" stroke-width="1.4" fill="none"/>
              <path d="M4 6l1.5 1.5L8 4.5" stroke="var(--ip-success-base)" stroke-width="1.4" stroke-linecap="round" fill="none"/>
            </svg>
            <span class="fc-text fc-text-strong">已发送</span>
          </div>
        </div>

        <!-- 中心聊天预览窗口 -->
        <div class="preview-stage">
          <div class="preview-stage-header">
            <span class="stage-dot"></span>
            <span class="stage-name">代码伙伴 · 默认项目</span>
            <span class="stage-meta">3 msgs</span>
          </div>
          <div class="preview-stage-body">
            <div class="msg user">
              <div class="av">我</div>
              <div class="bubble">
                帮我看看这个 <code>useChat</code> hook 的渲染次数为什么这么多？
              </div>
            </div>
            <div class="msg ai">
              <div class="av">码</div>
              <div>
                <div class="bubble">
                  看了你的代码，问题是 <code>messages</code> 数组每次都创建新引用，
                  <code>useEffect</code> 的依赖比较失败。<span class="cursor"></span>
                </div>
                <div class="msg-toolbar">
                  <span>
                    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
                      <path d="M5 1v8M2 6l3 3 3-3" stroke="currentColor" stroke-width="1.2" fill="none" stroke-linecap="round"/>
                    </svg>
                    复制
                  </span>
                  <span>
                    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
                      <path d="M7 1L3 5l4 4" stroke="currentColor" stroke-width="1.2" fill="none" stroke-linecap="round"/>
                    </svg>
                    重新生成
                  </span>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>

    <!-- ══════════════ 主内容容器 ══════════════ -->
    <div class="page">

      <!-- ─── Section 1: Buttons · 不对称双栏 ─── -->
      <div class="section-divider">
        <h2 class="h2">按钮 · <em>主角与配角</em></h2>
        <p class="lead">
          Primary 按钮应当真正承担"主角"地位 —— 放大、加阴影、独站一行。
          其它变体退到背景。
        </p>
      </div>

      <section class="buttons-section" id="components">
        <div class="buttons-stage">
          <h3>主按钮（默认态）</h3>
          <p class="stage-sub">高 52px · 圆角 10px · 冰蓝 500 + 12% 阴影</p>

          <div class="btn-display">
            <div class="label">button / md / primary</div>
            <Button
              variant="primary"
              size="lg"
              :class="{
                'demo-force-hover': stateView === 'hover',
                'demo-force-active': stateView === 'active',
              }"
              :loading="stateView === 'loading'"
              :disabled="stateView === 'disabled'"
            >
              <template #icon-left>
                <Plus :size="16" :stroke-width="2" />
              </template>
              创建 Agent
            </Button>
          </div>

          <div class="btn-state-row">
            <button
              v-for="s in ['default','hover','active','disabled','loading']"
              :key="s"
              type="button"
              class="btn-pill"
              :class="{ active: stateView === s }"
              @click="stateView = s as typeof stateView.value"
            >{{ s === 'default' ? '默认' : s === 'hover' ? '悬停' : s === 'active' ? '激活' : s === 'disabled' ? '禁用' : '加载中' }}</button>
          </div>
        </div>

        <div class="btn-matrix">
          <div
            v-for="variant in btnVariants"
            :key="variant.key"
            class="matrix-row"
          >
            <div class="row-label">
              {{ variant.label }}
              <span class="sub">{{ variant.sub }}</span>
            </div>
            <div class="row-items">
              <Button :variant="variant.key" size="sm">小尺寸</Button>
              <Button :variant="variant.key" size="md">默认</Button>
              <Button :variant="variant.key" size="lg">大尺寸</Button>
            </div>
          </div>
        </div>
      </section>

      <!-- ─── Section 2: Cards · 三栏不等比 ─── -->
      <div class="section-divider">
        <h2 class="h2">卡片 · <em>真实场景</em></h2>
        <p class="lead">
          演示三种不同质感的卡片：带封面图的项目卡、悬浮会话卡、统计指标卡。
          不用 12 个相同的占位符。
        </p>
      </div>

      <section class="cards-section">
        <!-- 主卡: 项目卡(含封面) -->
        <IpCard variant="shadow" padding="lg" interactive>
          <template #header>
            <div class="card-cover">
              <div class="paw-stamp">
                <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor" aria-hidden="true">
                  <circle cx="5" cy="6.5" r="2.4"/>
                  <circle cx="2.4" cy="3.6" r="1"/>
                  <circle cx="5" cy="2" r="1.1"/>
                  <circle cx="7.6" cy="3.6" r="1"/>
                </svg>
                Phase 2 项目
              </div>
            </div>
          </template>
          <div class="card-pad">
            <div class="card-meta-row">
              <span>2 天前更新</span>
              <span class="dot"></span>
              <span>3 个 Agent 团队</span>
              <span class="dot"></span>
              <span>12 次会话</span>
            </div>
            <h3 class="card-title">IcePaw 的 <em>项目维度</em> 设计</h3>
            <p class="card-desc">
              探索多 Agent 协作的项目架构：项目选择器、Agent 团队编排、跨项目会话管理。
            </p>
            <div class="team-stack">
              <div class="stack-av" style="background: linear-gradient(135deg, var(--ip-primary-500), var(--ip-primary-700));">主</div>
              <div class="stack-av" style="background: linear-gradient(135deg, var(--ip-success-base), var(--ip-success-active));">研</div>
              <div class="stack-av" style="background: linear-gradient(135deg, var(--ip-warning-base), var(--ip-warning-active));">审</div>
              <div class="stack-av more">+1</div>
              <span class="team-label">4 位 Agent · 在线</span>
            </div>
          </div>
        </IpCard>

        <!-- tinted 会话卡 -->
        <IpCard variant="filled" padding="md" class="real-card--tinted">
          <div class="card-pad">
            <div class="card-meta-row">
              <span class="meta-active">进行中</span>
              <span class="dot"></span>
              <span>18 分钟前</span>
            </div>
            <h3 class="card-title card-title-sm">
              调试 <em>useChat</em> hook
            </h3>
            <p class="card-desc card-desc-sm">
              messages 引用比较问题，可能要重构成 useReducer。
            </p>
            <div class="session-foot">
              <div class="stack-av stack-av-sm" style="background: linear-gradient(135deg, var(--ip-primary-500), var(--ip-primary-700));">码</div>
              <span class="session-foot-label">代码伙伴</span>
            </div>
          </div>
        </IpCard>

        <!-- metric 指标卡 -->
        <IpCard variant="bordered" padding="md" class="mini-card">
          <div class="icon-tile a">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
              <path d="M3 12L7 6l3 4 3-6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
          </div>
          <div class="mini-title">本月会话</div>
          <div class="mini-number">142</div>
          <div class="mini-meta">↑ 12% vs 上月</div>
        </IpCard>
      </section>

      <!-- ─── Section 3: Message · 单栏聊天场景 ─── -->
      <div class="section-divider">
        <h2 class="h2">消息 · <em>对话场景</em></h2>
        <p class="lead">
          8 条真实对话消息演示：用户、助手、系统、错误、流式、折叠。
        </p>
      </div>

      <section class="message-section">
        <div class="message-stage">
          <div class="preview-stage-header">
            <span class="stage-dot"></span>
            <span class="stage-name">代码伙伴 · 默认项目</span>
            <span class="stage-meta">实时</span>
          </div>
          <div class="preview-stage-body">
            <MessageBubble role="user" timestamp="10:30">
              帮我看看这段代码哪里有 bug？我本地跑不起来。
            </MessageBubble>

            <MessageBubble role="assistant" name="IcePaw" timestamp="10:30" meta="3.2s · 128 token">
              好的，看起来你的 <code>fetch</code> 调用缺少 <code>await</code>，所以返回的是 Promise 而不是实际数据。
              <template #footer-actions>
                <button type="button" class="ip-message__action-btn" aria-label="重新生成" @click="toastInfo">
                  <RotateCcw :size="14" :stroke-width="2" />
                </button>
              </template>
            </MessageBubble>

            <MessageBubble role="user" timestamp="10:31">明白了</MessageBubble>

            <MessageBubble role="user" timestamp="10:32">
              那如果是异步的，怎么同时发起多个请求？
            </MessageBubble>

            <MessageBubble role="assistant" name="IcePaw" timestamp="10:32" meta="2.1s · 95 token" streaming>
              推荐使用 <code>Promise.all</code>，它会并发地等待所有 Promise 完成，然后按顺序返回结果数组。
            </MessageBubble>

            <MessageBubble role="system">连接已断开，正在重连…</MessageBubble>

            <MessageBubble role="assistant" name="IcePaw" timestamp="10:33" error="连接超时">
              回复失败
            </MessageBubble>

            <MessageBubble role="assistant" name="IcePaw" timestamp="10:34" meta="5.4s · 312 token">
              推荐使用 <code>fetch</code> + <code>AbortController</code> 实现可中断的请求。
              配合 <code>useRef</code> 持有 controller 实例，在 cleanup 里调用 <code>abort()</code> 即可取消挂起的请求。
            </MessageBubble>
          </div>
        </div>
      </section>

      <!-- ─── Section 4: Inputs · 非对称 inline 表单 ─── -->
      <div class="section-divider">
        <h2 class="h2">输入 · <em>表单场景</em></h2>
        <p class="lead">
          表单 + 辅助控件：尺寸、状态、前缀后缀、可清除、错误抖动。
        </p>
      </div>

      <section class="inputs-section">
        <!-- 左侧: 表单舞台 -->
        <div class="inputs-form">
          <h3>创建 Agent</h3>
          <p class="stage-sub">真实表单场景：用户名 + 描述 + 模型</p>

          <div class="form-row">
            <label>Agent 名称</label>
            <Input v-model="agentName" placeholder="例如：代码伙伴" />
          </div>

          <div class="form-row">
            <label>提示词</label>
            <Textarea v-model="agentPrompt" placeholder="你是一个乐于助人的助手…" :rows="3" />
          </div>

          <div class="form-row">
            <label>模型</label>
            <IpSelect v-model="selectValue" :options="modelOptions" placeholder="选择模型" />
          </div>

          <div class="form-row form-row--cta">
            <Button variant="ghost">取消</Button>
            <Button variant="primary" @click="toastSuccess">保存</Button>
          </div>
        </div>

        <!-- 右侧: 辅助态 -->
        <div class="inputs-aux">
          <h3>辅助态</h3>

          <span class="aux-label">尺寸</span>
          <div class="aux-stack">
            <Input v-model="auxValue" size="sm" placeholder="小号" />
            <Input v-model="auxValue" size="md" placeholder="中号（默认）" />
            <Input v-model="auxValue" size="lg" placeholder="大号" />
          </div>

          <span class="aux-label">前缀 / 后缀</span>
          <div class="aux-stack">
            <Input v-model="auxValue" placeholder="搜索…">
              <template #prefix><Search :size="14" /></template>
            </Input>
            <Input :model-value="'user'" placeholder="用户名">
              <template #prefix><AtSign :size="14" /></template>
            </Input>
            <Input :model-value="'#general'" placeholder="频道">
              <template #prefix><Hash :size="14" /></template>
            </Input>
            <Input :model-value="'user'" placeholder="用户名">
              <template #suffix>
                <span class="aux-suffix-text">@icepaw.dev</span>
              </template>
            </Input>
          </div>

          <span class="aux-label">可清除</span>
          <Input v-model="auxValue" placeholder="搜索会话…" clearable />

          <span class="aux-label">错误抖动</span>
          <div class="aux-row">
            <Button variant="secondary" size="sm" @click="showInputError">触发错误</Button>
            <Button variant="ghost" size="sm" @click="clearInputError">清除</Button>
          </div>

          <span class="aux-label">禁用 / 只读</span>
          <div class="aux-stack">
            <Input :model-value="'禁用字段'" disabled />
            <Input :model-value="'只读字段'" readonly />
          </div>

          <span class="aux-label">错误提示（带消息）</span>
          <div class="aux-row">
            <Input
              :model-value="'zhangsan'"
              :error="inputError"
              :error-message="inputErrorMessage"
              placeholder="用户名"
            />
            <Button variant="secondary" size="sm" @click="showInputError">触发错误</Button>
          </div>
        </div>
      </section>

      <!-- ─── Section 5: Modal · 双联对照 ─── -->
      <div class="section-divider">
        <h2 class="h2">模态 · <em>三个尺寸</em></h2>
        <p class="lead">
          400 / 560 / 720 三档宽度，覆盖确认、编辑、详情场景。
        </p>
      </div>

      <section class="modals-section">
        <div class="modal-triggers">
          <h3>三档尺寸</h3>
          <p class="stage-sub">400 / 560 / 720 — 覆盖确认、编辑、详情</p>

          <div class="trigger-stack">
            <Button variant="primary" size="sm" @click="modalOpenSm = true">
              打开（400px） · 确认操作
            </Button>
            <Button variant="primary" size="md" @click="modalOpenMd = true">
              打开（560px） · 编辑表单
            </Button>
            <Button variant="primary" size="lg" @click="modalOpenLg = true">
              打开（720px） · 详情视图
            </Button>
          </div>

          <Modal v-model="modalOpenSm" title="删除会话" size="sm">
            <p class="modal-body-text">将永久删除该会话及其所有消息，无法恢复。</p>
            <template #footer>
              <Button variant="ghost" @click="modalOpenSm = false">取消</Button>
              <Button variant="danger" @click="modalOpenSm = false">删除</Button>
            </template>
          </Modal>

          <Modal v-model="modalOpenMd" title="编辑代理" size="md">
            <Input :model-value="'通用助手'" placeholder="代理名称" class="modal-form-field" />
            <Textarea :model-value="'你是一个乐于助人的助手。三思而后行。'" placeholder="…" :rows="4" />
            <template #footer>
              <Button variant="ghost" @click="modalOpenMd = false">取消</Button>
              <Button variant="primary" @click="modalOpenMd = false">保存</Button>
            </template>
          </Modal>

          <Modal v-model="modalOpenLg" title="系统提示词" size="lg">
            <Textarea v-model="taValue" placeholder="你是一个乐于助人的助手…" :rows="6" />
            <template #footer>
              <Button variant="ghost" @click="modalOpenLg = false">取消</Button>
              <Button variant="primary" @click="modalOpenLg = false">保存</Button>
            </template>
          </Modal>
        </div>

        <div class="modal-preview">
          <h3>Modal 解构</h3>
          <p class="stage-sub">静态预览 — 不触发，用于展示结构</p>

          <div class="modal-anatomy">
            <div class="anatomy-row anatomy-row--header">
              <div class="anatomy-label">标题</div>
              <div class="anatomy-close" aria-hidden="true">×</div>
            </div>

            <hr class="anatomy-divider" />

            <div class="anatomy-body">
              <p>正文内容。Modal 的核心是聚焦点 —— 用户的所有注意力在这里。</p>
              <p class="anatomy-body-meta">内容应控制在 720px 宽度以内。超过这个宽度，建议改用侧滑面板或独立页面。</p>
            </div>

            <hr class="anatomy-divider" />

            <div class="anatomy-row anatomy-row--footer">
              <Button variant="ghost">取消</Button>
              <Button variant="primary">确认</Button>
            </div>
          </div>

          <ul class="anatomy-list">
            <li><strong>Header</strong> · 标题 + 关闭按钮</li>
            <li><strong>Body</strong> · 主要内容，最大 720px</li>
            <li><strong>Footer</strong> · 主要操作右对齐</li>
            <li><strong>遮罩</strong> · rgba(20, 24, 31, 0.5) 暗化背景</li>
          </ul>
        </div>
      </section>

      <!-- ─── Section 6: Toast · 横向时间线 ─── -->
      <div class="section-divider">
        <h2 class="h2">提示 · <em>四类反馈</em></h2>
        <p class="lead">
          成功 / 信息 / 警告 / 错误 + 合并策略。
        </p>
      </div>

      <section class="toast-section">
        <div class="toast-timeline">
          <div class="toast-step">
            <div class="toast-step-marker success"></div>
            <div class="toast-step-content">
              <h4>成功 · Success</h4>
              <p>保存、发送等正向操作反馈。</p>
              <code>toast.success('保存成功')</code>
            </div>
            <Button variant="primary" @click="toastSuccess">触发</Button>
          </div>

          <div class="toast-step">
            <div class="toast-step-marker info"></div>
            <div class="toast-step-content">
              <h4>信息 · Info</h4>
              <p>中性通知，不需要立即动作。</p>
              <code>toast.info('检测到新版本')</code>
            </div>
            <Button variant="secondary" @click="toastInfo">触发</Button>
          </div>

          <div class="toast-step">
            <div class="toast-step-marker warning"></div>
            <div class="toast-step-content">
              <h4>警告 · Warning</h4>
              <p>非阻塞问题，需要用户注意。</p>
              <code>toast.warning('网络连接不稳定')</code>
            </div>
            <Button variant="primary" @click="toastWarning">触发</Button>
          </div>

          <div class="toast-step">
            <div class="toast-step-marker error"></div>
            <div class="toast-step-content">
              <h4>错误 · Error</h4>
              <p>操作失败，需要修复或重试。</p>
              <code>toast.error('保存失败')</code>
            </div>
            <Button variant="danger" @click="toastError">触发</Button>
          </div>
        </div>

        <div class="toast-merge">
          <h4>合并策略</h4>
          <p>同类型连续提示自动合并为一条，计时重置。</p>
          <Button variant="primary" @click="toastMergeDemo">连续触发 3 次</Button>
        </div>
      </section>

      <!-- ─── Section 7: Avatar · 3 行布局 ─── -->
      <div class="section-divider">
        <h2 class="h2">头像 · <em>多场景</em></h2>
        <p class="lead">
          6 档尺寸 × 4 种内容类型 × 2 种形状 + 上传态。
        </p>
      </div>

      <section class="avatar-section">
        <div class="avatar-size-strip">
          <span class="strip-label">尺寸档 · 6 档</span>
          <IpFlex size="md" align="center">
            <IpAvatar :source="{ type: 'initials', text: 'A', bgColor: 'var(--ip-primary-500)' }" size="xs" />
            <IpAvatar :source="{ type: 'initials', text: 'B', bgColor: 'var(--ip-primary-500)' }" size="sm" />
            <IpAvatar :source="{ type: 'initials', text: 'C', bgColor: 'var(--ip-primary-500)' }" size="md" />
            <IpAvatar :source="{ type: 'initials', text: 'D', bgColor: 'var(--ip-primary-500)' }" size="lg" />
            <IpAvatar :source="{ type: 'initials', text: 'E', bgColor: 'var(--ip-primary-500)' }" size="xl" />
            <IpAvatar :source="{ type: 'initials', text: 'F', bgColor: 'var(--ip-primary-500)' }" size="xxl" />
          </IpFlex>
          <span class="strip-meta">xs=20px · sm=28px · md=36px · lg=48px · xl=64px · xxl=96px</span>
        </div>

        <div class="avatar-type-row">
          <span class="strip-label">内容类型 · 4 种</span>
          <IpFlex size="lg" align="center">
            <div class="avatar-type">
              <IpAvatar v-if="avatarSrc" :source="{ type: 'image', src: avatarSrc, alt: '图片' }" size="lg" />
              <IpAvatar v-else :source="{ type: 'default' }" size="lg" />
              <span class="type-name">image</span>
              <span class="type-desc">网络或本地图片</span>
            </div>
            <div class="avatar-type">
              <IpAvatar :source="{ type: 'initials', text: 'ZP', bgColor: 'var(--ip-info-base)', fgColor: 'var(--ip-white)' }" size="lg" />
              <span class="type-name">initials</span>
              <span class="type-desc">首字母 + 自定义色</span>
            </div>
            <div class="avatar-type">
              <IpAvatar :source="{ type: 'icon', icon: Bot, color: 'var(--ip-color-icon-default)' }" size="lg" />
              <span class="type-name">icon</span>
              <span class="type-desc">Lucide 图标</span>
            </div>
            <div class="avatar-type">
              <IpAvatar :source="{ type: 'default' }" size="lg" />
              <span class="type-name">default</span>
              <span class="type-desc">降级占位</span>
            </div>
          </IpFlex>
        </div>

        <div class="avatar-shape-row">
          <span class="strip-label">形状 + 上传态 · 5 种</span>
          <IpFlex size="lg" align="center">
            <div class="avatar-type">
              <IpAvatar :source="{ type: 'initials', text: 'R', bgColor: 'var(--ip-warning-base)' }" size="lg" shape="rounded" />
              <span class="type-name">rounded</span>
              <span class="type-desc">默认 · 项目头像</span>
            </div>
            <div class="avatar-type">
              <IpAvatar :source="{ type: 'initials', text: 'C', bgColor: 'var(--ip-success-base)' }" size="lg" shape="circle" />
              <span class="type-name">circle</span>
              <span class="type-desc">用户头像</span>
            </div>
            <div class="avatar-type">
              <IpAvatar
                v-if="avatarSrc"
                :source="{ type: 'image', src: avatarSrc }"
                size="lg"
                :uploadable="true"
                @upload="onAvatarUpload"
                @upload-error="onAvatarError"
              />
              <IpAvatar v-else :source="{ type: 'default' }" size="lg" />
              <span class="type-name">uploadable</span>
              <span class="type-desc">hover 相机蒙层</span>
            </div>
            <div class="avatar-type">
              <IpAvatar
                v-if="avatarSrc"
                :source="{ type: 'image', src: avatarSrc }"
                size="lg"
                :uploadable="true"
                :removable="true"
                @remove="clearAvatar"
              />
              <IpAvatar v-else :source="{ type: 'default' }" size="lg" />
              <span class="type-name">removable</span>
              <span class="type-desc">X 按钮清除</span>
            </div>
            <div class="avatar-type">
              <IpAvatar
                v-if="avatarSrc"
                :source="{ type: 'image', src: avatarSrc }"
                size="lg"
                :uploadable="true"
                :loading="true"
              />
              <IpAvatar v-else :source="{ type: 'default' }" size="lg" :loading="true" />
              <span class="type-name">loading</span>
              <span class="type-desc">spinner 状态</span>
            </div>
          </IpFlex>
        </div>
      </section>

      <!-- ─── Section 8: Select · 带描述的选项 ─── -->
      <div class="section-divider">
        <h2 class="h2">选择 · <em>带描述的选项</em></h2>
        <p class="lead">
          模型选择器模拟场景：icon + label + description + disabled。
        </p>
      </div>

      <section class="select-section">
        <div class="select-stage">
          <h3>模型选择器</h3>
          <p class="stage-sub">icon + label + description + disabled 的组合场景</p>

          <div class="form-row">
            <label>主模型</label>
            <IpSelect v-model="selectValue" :options="modelOptions" />
          </div>

          <div class="form-row">
            <label>语气</label>
            <IpSelect v-model="selectTone" :options="toneOptions" />
          </div>

          <div class="form-row">
            <label>带错误态</label>
            <IpSelect
              v-model="selectValue"
              :options="modelOptions"
              :error="selectError"
              :error-message="selectErrorMessage"
            />
            <Button
              variant="ghost"
              size="sm"
              class="select-error-trigger"
              @click="triggerSelectError"
            >
              {{ selectError ? '清除错误' : '触发错误' }}
            </Button>
          </div>

          <div class="form-row">
            <label>可清除</label>
            <IpSelect v-model="selectClearable" :options="modelOptions" clearable />
          </div>
        </div>
      </section>

      <!-- ─── Section 9: Empty · 四种差异化空状态 ─── -->
      <div class="section-divider">
        <h2 class="h2">空状态 · <em>四种表达</em></h2>
        <p class="lead">
          探索 / 实用 / 引导 / 专业 —— 不同空态不同情绪。
        </p>
      </div>

      <section class="empty-section">
        <div class="empty-grid">
          <!-- 探索型 · 引导用户开始 -->
          <IpEmptyState
            :icon="Plus"
            title="开始第一个项目"
            description="项目是 IcePaw 的一等公民。在项目里组织 Agent 团队、管理会话。"
            :primary-action="{ label: '创建项目' }"
            :secondary-action="{ label: '浏览模板' }"
          />

          <!-- 实用型 · 任务已完成 -->
          <IpEmptyState
            :icon="CheckCircle"
            title="所有会话已读"
            description="12 个会话 · 0 个未读"
            compact
            :centered="false"
            :primary-action="{ label: '查看历史' }"
          />

          <!-- 引导型 · 步骤列表 -->
          <IpEmptyState title="5 分钟上手 IcePaw" :centered="false">
            <ol class="empty-steps">
              <li><span class="step-num">1</span> 选择或创建项目</li>
              <li><span class="step-num">2</span> 配置 Agent 团队</li>
              <li><span class="step-num">3</span> 开始第一次对话</li>
            </ol>
            <template #primary-action>
              <Button variant="primary" block>继续</Button>
            </template>
          </IpEmptyState>

          <!-- 专业型 · 无权限 / 错误 -->
          <IpEmptyState
            :icon="AlertCircle"
            title="无法访问此项目"
            description="权限不足或项目已归档"
            :centered="false"
            :primary-action="{ label: '申请权限' }"
            :secondary-action="{ label: '联系管理员' }"
          />
        </div>
      </section>

      <!-- ─── Section 10: Dropdown · 命令面板 ─── -->
      <div class="section-divider">
        <h2 class="h2">菜单 · <em>命令面板</em></h2>
        <p class="lead">
          shortcut + section + danger 的完整命令面板形态。
        </p>
      </div>

      <section class="dropdown-section">
        <div class="dropdown-grid">
          <!-- 基础菜单 -->
          <div class="dropdown-demo-block">
            <h4>基础菜单</h4>
            <p class="stage-sub">icon + divider + danger</p>
            <IpDropdownMenu v-model="ddOpen" :items="ddItems">
              <template #trigger>
                <Button variant="secondary" size="sm">
                  <template #icon-right><MoreHorizontal :size="14" /></template>
                  操作
                </Button>
              </template>
            </IpDropdownMenu>
          </div>

          <!-- 完整菜单（含分组+shortcut） -->
          <div class="dropdown-demo-block">
            <h4>完整菜单</h4>
            <p class="stage-sub">label + shortcut + 多分组</p>
            <IpDropdownMenu v-model="ddOpenFull" :items="ddFullItems">
              <template #trigger>
                <Button variant="secondary" size="sm">
                  <template #icon-right><ChevronDown :size="14" /></template>
                  更多选项
                </Button>
              </template>
            </IpDropdownMenu>
          </div>

          <!-- 受限菜单（disabled 项） -->
          <div class="dropdown-demo-block">
            <h4>受限菜单</h4>
            <p class="stage-sub">disabled + danger</p>
            <IpDropdownMenu v-model="ddOpenDisabled" :items="ddDisabledItems">
              <template #trigger>
                <Button variant="secondary" size="sm">
                  <template #icon-right><MoreHorizontal :size="14" /></template>
                  受限操作
                </Button>
              </template>
            </IpDropdownMenu>
          </div>

          <!-- hover 触发 -->
          <div class="dropdown-demo-block">
            <h4>Hover 触发</h4>
            <p class="stage-sub">triggerAction="hover"</p>
            <IpDropdownMenu v-model="ddOpenHover" :items="ddItems" trigger-action="hover">
              <template #trigger>
                <Button variant="secondary" size="sm">
                  <template #icon-right><MoreHorizontal :size="14" /></template>
                  悬停打开
                </Button>
              </template>
            </IpDropdownMenu>
          </div>
        </div>
      </section>

      <!-- ─── Section 11: Popconfirm · 危险操作确认 ─── -->
      <div class="section-divider">
        <h2 class="h2">确认 · <em>危险操作</em></h2>
        <p class="lead">
          4 个方向定位 + loading + danger 反馈。
        </p>
      </div>

      <section class="popconfirm-section">
        <div class="popconfirm-stage">
          <h3>四个方向 + loading + danger</h3>
          <p class="stage-sub">点击按钮触发确认气泡，危险操作有 loading 反馈</p>

          <div class="pop-grid">
            <div class="pop-item">
              <IpPopconfirm v-model="popTop" title="上方确认" placement="top" @confirm="toastSuccess">
                确认删除这个会话吗？
                <template #trigger>
                  <Button variant="secondary" size="sm">上方</Button>
                </template>
              </IpPopconfirm>
            </div>

            <div class="pop-item">
              <IpPopconfirm v-model="popBottom" title="下方确认" placement="bottom" @confirm="toastSuccess">
                确认删除这个会话吗？
                <template #trigger>
                  <Button variant="secondary" size="sm">下方（默认）</Button>
                </template>
              </IpPopconfirm>
            </div>

            <div class="pop-item">
              <IpPopconfirm v-model="popLeft" title="左侧确认" placement="left" @confirm="toastSuccess">
                确认删除这个会话吗？
                <template #trigger>
                  <Button variant="secondary" size="sm">左侧</Button>
                </template>
              </IpPopconfirm>
            </div>

            <div class="pop-item">
              <IpPopconfirm v-model="popRight" title="右侧确认" placement="right" @confirm="toastSuccess">
                确认删除这个会话吗？
                <template #trigger>
                  <Button variant="secondary" size="sm">右侧</Button>
                </template>
              </IpPopconfirm>
            </div>

            <div class="pop-item pop-item--wide">
              <h4>危险操作 + loading 反馈</h4>
              <IpPopconfirm
                v-model="popDangerOpen"
                title="删除会话"
                :danger="true"
                :loading="popLoading"
                @confirm="confirmDelete"
              >
                将永久删除该会话及其所有消息。
                <template #trigger>
                  <Button variant="danger">删除会话</Button>
                </template>
              </IpPopconfirm>
            </div>
          </div>
        </div>
      </section>

      <!-- ─── Section 12: Flex · 轴线矩阵 ─── -->
      <div class="section-divider">
        <h2 class="h2">弹性 · <em>轴线矩阵</em></h2>
        <p class="lead">
          gap × direction × justify × align 全矩阵对照。
        </p>
      </div>

      <section class="flex-section">
        <div class="flex-matrix">
          <div class="matrix-block">
            <h4>方向 · direction</h4>
            <IpFlex direction="row" size="sm">
              <span class="chip">A</span><span class="chip">B</span><span class="chip">C</span>
            </IpFlex>
            <IpFlex direction="row-reverse" size="sm">
              <span class="chip">A</span><span class="chip">B</span><span class="chip">C</span>
            </IpFlex>
            <IpFlex direction="column" size="sm">
              <span class="chip">A</span><span class="chip">B</span><span class="chip">C</span>
            </IpFlex>
            <IpFlex direction="column-reverse" size="sm">
              <span class="chip">A</span><span class="chip">B</span><span class="chip">C</span>
            </IpFlex>
          </div>

          <div class="matrix-block">
            <h4>gap · 5 档</h4>
            <div class="gap-row">
              <code>xs · 8px</code>
              <IpFlex size="xs"><span class="chip">A</span><span class="chip">B</span></IpFlex>
            </div>
            <div class="gap-row">
              <code>sm · 12px</code>
              <IpFlex size="sm"><span class="chip">A</span><span class="chip">B</span></IpFlex>
            </div>
            <div class="gap-row">
              <code>md · 16px</code>
              <IpFlex size="md"><span class="chip">A</span><span class="chip">B</span></IpFlex>
            </div>
            <div class="gap-row">
              <code>lg · 24px</code>
              <IpFlex size="lg"><span class="chip">A</span><span class="chip">B</span></IpFlex>
            </div>
            <div class="gap-row">
              <code>xl · 32px</code>
              <IpFlex size="xl"><span class="chip">A</span><span class="chip">B</span></IpFlex>
            </div>
          </div>

          <div class="matrix-block">
            <h4>justify · 主轴对齐</h4>
            <IpFlex justify="start" size="sm" class="justify-track">
              <span class="chip">A</span><span class="chip">B</span>
            </IpFlex>
            <IpFlex justify="center" size="sm" class="justify-track">
              <span class="chip">A</span><span class="chip">B</span>
            </IpFlex>
            <IpFlex justify="end" size="sm" class="justify-track">
              <span class="chip">A</span><span class="chip">B</span>
            </IpFlex>
            <IpFlex justify="space-between" size="sm" class="justify-track">
              <span class="chip">A</span><span class="chip">B</span>
            </IpFlex>
            <IpFlex justify="space-around" size="sm" class="justify-track">
              <span class="chip">A</span><span class="chip">B</span>
            </IpFlex>
          </div>

          <div class="matrix-block">
            <h4>align · 交叉轴</h4>
            <div class="align-track">
              <IpFlex align="start" size="sm" class="align-row">
                <span class="chip chip-tall">高</span><span class="chip">矮</span>
              </IpFlex>
            </div>
            <div class="align-track">
              <IpFlex align="center" size="sm" class="align-row">
                <span class="chip chip-tall">高</span><span class="chip">矮</span>
              </IpFlex>
            </div>
            <div class="align-track">
              <IpFlex align="end" size="sm" class="align-row">
                <span class="chip chip-tall">高</span><span class="chip">矮</span>
              </IpFlex>
            </div>
          </div>
        </div>
      </section>

      <!-- ─── Section 13: Container · 宽度阶梯 ─── -->
      <div class="section-divider">
        <h2 class="h2">容器 · <em>宽度阶梯</em></h2>
        <p class="lead">
          sm / md / lg / xl + fluid + centered 组合。
        </p>
      </div>

      <section class="container-section">
        <div class="container-stage">
          <h3>sm / md / lg / xl 阶梯</h3>
          <p class="stage-sub">默认 md（720px）与消息区对齐</p>

          <IpContainer max-width="sm" padding-x="md" padding-y="sm">
            <div class="container-frame">小 · 480px</div>
          </IpContainer>

          <IpContainer max-width="md" padding-x="md" padding-y="sm">
            <div class="container-frame">中 · 720px（默认）</div>
          </IpContainer>

          <IpContainer max-width="lg" padding-x="md" padding-y="sm">
            <div class="container-frame">大 · 960px</div>
          </IpContainer>

          <IpContainer max-width="xl" padding-x="md" padding-y="sm">
            <div class="container-frame">特大 · 1200px</div>
          </IpContainer>
        </div>

        <div class="container-extras">
          <h4>居中 · centered</h4>
          <div class="extra-pair">
            <IpContainer max-width="md" centered padding-x="md" padding-y="sm">
              <div class="container-frame container-frame--small">居中 · 启用</div>
            </IpContainer>
            <div class="extra-context">
              <code>centered={true}</code>
              <p>默认行为，子内容居中显示。</p>
            </div>
          </div>

          <h4>流体 · fluid</h4>
          <div class="extra-pair">
            <IpContainer max-width="md" fluid padding-x="md" padding-y="sm">
              <div class="container-frame container-frame--fluid">fluid · 忽略最大宽度</div>
            </IpContainer>
            <div class="extra-context">
              <code>fluid</code>
              <p>无视 max-width 限制，用于全宽横幅。</p>
            </div>
          </div>

          <h4>内边距预设</h4>
          <div class="extra-pair">
            <IpContainer max-width="md" padding-x="xs" padding-y="xs">
              <div class="container-frame">padding xs</div>
            </IpContainer>
            <div class="extra-context">
              <code>padding-x="xs"</code>
              <p>4 种预设：xs/sm/md/lg</p>
            </div>
          </div>
        </div>
      </section>

      <!-- ══════════════ Footer ══════════════ -->
      <footer class="footer">
        <div class="footer-brand-col">
          <div class="footer-brand">IcePaw Design System</div>
          <div class="footer-tag">
            为多 Agent 项目协作打造的对话界面组件库。
            冰蓝色调 · 极地意象 · 克制表达。
          </div>
          <div class="footer-snow">
            <svg viewBox="0 0 12 12" fill="none" aria-hidden="true">
              <path d="M6 1v10M1 6h10M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1" stroke-linecap="round"/>
            </svg>
            <svg viewBox="0 0 12 12" fill="none" aria-hidden="true">
              <path d="M6 1v10M1 6h10M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1" stroke-linecap="round"/>
            </svg>
            <svg viewBox="0 0 12 12" fill="none" aria-hidden="true">
              <path d="M6 1v10M1 6h10M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1" stroke-linecap="round"/>
            </svg>
            <span class="footer-snow-text">crafted for polar clarity</span>
          </div>
        </div>

        <div class="footer-col">
          <h4>组件</h4>
          <a href="#components">Buttons</a>
          <a href="#components">Messages</a>
          <a href="#components">Inputs</a>
          <a href="#components">Modals</a>
          <a href="#components">Empty States</a>
        </div>

        <div class="footer-col">
          <h4>资源</h4>
          <a href="#components">组件</a>
          <a href="#">Figma 文件</a>
          <a href="#">图标库</a>
          <a href="#">动效指南</a>
        </div>

        <div class="footer-col">
          <h4>项目</h4>
          <a href="#">GitHub</a>
          <a href="#">更新日志</a>
          <a href="#">路线图</a>
          <a href="#">反馈</a>
        </div>
      </footer>
    </div>
  </div>
</template>

<style scoped>
/* ================================================================
   IcePaw UI · Preview v2 — Brand Showcase Landing
   规范: wave2-preview-site-spec.md
   ================================================================ */

/* ── Global reset ── */
* {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

html {
  -webkit-text-size-adjust: 100%;
  tab-size: 4;
}

body {
  font-family: var(--ip-font-sans);
  font-size: var(--ip-text-body-size);
  line-height: var(--ip-line-height-loose2);
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-primary);
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  text-rendering: optimizeLegibility;
}

[data-theme='dark'] body {
  background: var(--ip-color-bg-primary);
  color: var(--ip-white);
}

.preview-root {
  min-height: 100vh;
  background: var(--ip-color-bg-primary);
  color: inherit;
  overflow-x: hidden;
}

/* ── Page container ── */
.page {
  max-width: 1280px;
  margin: 0 auto;
  padding: 0 var(--ip-spacing-8);
}

.page.hero {
  padding: var(--ip-spacing-20) var(--ip-spacing-8) var(--ip-spacing-16);
}

/* ================================================================
   Top nav
   ================================================================ */
.nav {
  position: sticky;
  top: 0;
  z-index: var(--ip-z-sticky);
  height: 60px;
  background: rgba(250, 251, 252, 0.85);
  backdrop-filter: blur(12px);
  -webkit-backdrop-filter: blur(12px);
  border-bottom: 1px solid var(--ip-gray-200);
  display: flex;
  align-items: center;
  padding: 0 var(--ip-spacing-8);
}

[data-theme='dark'] .nav {
  background: rgba(20, 24, 31, 0.85);
  border-bottom-color: var(--ip-gray-800);
}

.nav-inner {
  max-width: 1280px;
  margin: 0 auto;
  width: 100%;
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-8);
}

.brand {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  font-weight: var(--ip-font-weight-semibold);
  font-size: 15px;
  color: var(--ip-color-text-primary);
  text-decoration: none;
}

[data-theme='dark'] .brand { color: var(--ip-white); }

.brand-mark {
  width: 28px;
  height: 28px;
  border-radius: var(--ip-radius-sm);
  display: grid;
  place-items: center;
  box-shadow: 0 1px 2px rgba(20, 24, 31, 0.04);
}

.brand-mark svg { width: 26px; height: 26px; display: block; }

.brand .accent {
  color: var(--ip-primary-700);
  font-weight: var(--ip-font-weight-regular);
  margin-left: 2px;
}

.nav-links {
  display: flex;
  align-items: center;
  gap: 2px;
  margin-left: var(--ip-spacing-6);
}

.nav-link {
  font-size: 13px;
  color: var(--ip-color-text-body);
  padding: 6px 12px;
  border-radius: var(--ip-radius-md);
  text-decoration: none;
  transition: all var(--ip-duration-base) var(--ip-ease-out);
}

.nav-link:hover {
  background: var(--ip-gray-100);
  color: var(--ip-color-text-primary);
}

.nav-link.active {
  color: var(--ip-primary-700);
  background: var(--ip-primary-50);
}

[data-theme='dark'] .nav-link.active {
  background: rgba(70, 128, 194, 0.15);
}

.nav-right {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-3);
}

.nav-link-quiet {
  font-size: 13px;
  color: var(--ip-color-text-tertiary);
  text-decoration: none;
}

.nav-link-quiet:hover { color: var(--ip-color-text-primary); }

.btn-sm {
  height: 32px;
  padding: 0 14px;
  border-radius: var(--ip-radius-md);
  font-size: 13px;
  font-weight: var(--ip-font-weight-medium);
  border: none;
  cursor: pointer;
  font-family: var(--ip-font-sans);
  display: inline-flex;
  align-items: center;
  gap: 6px;
  transition: all var(--ip-duration-base) var(--ip-ease-out);
}

.btn-sm.primary {
  background: var(--ip-primary-500);
  color: var(--ip-white);
}

.btn-sm.primary:hover { background: var(--ip-primary-600); }

.theme-toggle {
  width: 32px;
  height: 32px;
  border-radius: var(--ip-radius-md);
  border: none;
  background: transparent;
  cursor: pointer;
  display: grid;
  place-items: center;
  color: var(--ip-color-text-tertiary);
  transition: all var(--ip-duration-base) var(--ip-ease-out);
}

.theme-toggle:hover {
  background: var(--ip-gray-100);
  color: var(--ip-color-text-primary);
}

[data-theme='dark'] .theme-toggle:hover {
  background: var(--ip-gray-800);
}

/* ================================================================
   HERO
   ================================================================ */
.hero {
  padding: var(--ip-spacing-20) 0 var(--ip-spacing-16);
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--ip-spacing-12);
  align-items: center;
  position: relative;
}

.hero-text { max-width: 540px; }

.hero-eyebrow {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: 4px 10px 4px 4px;
  background: var(--ip-white);
  border: 1px solid var(--ip-gray-200);
  border-radius: var(--ip-radius-full);
  font-size: 12px;
  color: var(--ip-color-text-body);
  margin-bottom: var(--ip-spacing-6);
  box-shadow: var(--ip-shadow-xs);
}

[data-theme='dark'] .hero-eyebrow {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
  color: var(--ip-gray-300);
}

.hero-eyebrow .pulse {
  width: 18px;
  height: 18px;
  background: var(--ip-primary-100);
  color: var(--ip-primary-700);
  border-radius: var(--ip-radius-full);
  display: grid;
  place-items: center;
  font-family: var(--ip-font-mono);
  font-size: 10px;
  font-weight: var(--ip-font-weight-semibold);
}

.hero h1 {
  font-family: var(--ip-font-display);
  font-weight: var(--ip-font-weight-regular);
  font-size: clamp(2.8rem, 5.6vw, 4.4rem);
  line-height: 1.02;
  letter-spacing: -0.03em;
  color: var(--ip-color-text-primary);
  text-wrap: balance;
  margin-bottom: var(--ip-spacing-5);
}

[data-theme='dark'] .hero h1 { color: var(--ip-white); }

.hero h1 em {
  font-style: italic;
  color: var(--ip-primary-600);
}

.hero-sub {
  font-size: 17px;
  line-height: 1.55;
  color: var(--ip-color-text-body);
  margin-bottom: var(--ip-spacing-8);
  max-width: 480px;
}

[data-theme='dark'] .hero-sub { color: var(--ip-gray-300); }

.hero-cta { display: flex; align-items: center; gap: var(--ip-spacing-3); }

.hero-cta .btn-lg {
  height: 46px;
  padding: 0 var(--ip-spacing-6);
  font-size: 14px;
  font-weight: var(--ip-font-weight-medium);
  border-radius: var(--ip-radius-md);
  border: none;
  cursor: pointer;
  font-family: var(--ip-font-sans);
  display: inline-flex;
  align-items: center;
  gap: 8px;
  transition: all var(--ip-duration-message) var(--ip-ease-out);
  text-decoration: none;
}

.hero-cta .btn-lg.primary {
  background: var(--ip-primary-500);
  color: var(--ip-white);
  box-shadow: 0 2px 10px rgba(70, 128, 194, 0.3);
}

.hero-cta .btn-lg.primary:hover {
  background: var(--ip-primary-600);
  transform: translateY(-1px);
  box-shadow: 0 6px 18px rgba(70, 128, 194, 0.4);
}

.hero-cta .btn-lg.ghost {
  background: transparent;
  color: var(--ip-color-text-primary);
  border: 1px solid var(--ip-gray-300);
}

.hero-cta .btn-lg.ghost:hover {
  background: var(--ip-gray-100);
  border-color: var(--ip-gray-500);
}

[data-theme='dark'] .hero-cta .btn-lg.ghost {
  color: var(--ip-white);
  border-color: var(--ip-gray-700);
}

.hero-meta {
  margin-top: var(--ip-spacing-8);
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-5);
  color: var(--ip-color-text-tertiary);
  font-size: 12px;
  flex-wrap: wrap;
}

.hero-meta .dot {
  width: 4px;
  height: 4px;
  background: var(--ip-gray-300);
  border-radius: var(--ip-radius-full);
}

.hero-meta a {
  color: var(--ip-color-text-body);
  text-decoration: none;
  border-bottom: 1px dotted var(--ip-gray-300);
  padding-bottom: 1px;
  font-family: var(--ip-font-mono);
}

.hero-meta a:hover {
  color: var(--ip-primary-700);
  border-color: var(--ip-primary-400);
}

/* ---- Hero right column ---- */
.hero-visual {
  position: relative;
  height: 480px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.crystal-bg {
  position: absolute;
  inset: 0;
  display: grid;
  place-items: center;
  opacity: 0.35;
  pointer-events: none;
}

.crystal-bg svg { width: 100%; height: 100%; }

.float-card {
  position: absolute;
  background: var(--ip-white);
  border: 1px solid var(--ip-gray-200);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-lg);
  padding: var(--ip-spacing-3);
  z-index: 2;
  animation: float-in 600ms var(--ip-ease-out) backwards;
}

[data-theme='dark'] .float-card {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

.float-card.f1 {
  top: 30px; left: 10px;
  --rot: -3deg;
  animation-delay: 200ms;
}
.float-card.f2 {
  bottom: 50px; left: 0;
  --rot: 2deg;
  animation-delay: 350ms;
}
.float-card.f3 {
  top: 60px; right: 20px;
  --rot: 4deg;
  animation-delay: 500ms;
}
.float-card.f4 {
  bottom: 30px; right: 10px;
  --rot: -2deg;
  animation-delay: 650ms;
}

@keyframes float-in {
  from { opacity: 0; transform: translateY(8px) rotate(var(--rot, 0deg)); }
  to { opacity: 1; transform: translateY(0) rotate(var(--rot, 0deg)); }
}

@media (prefers-reduced-motion: reduce) {
  .float-card {
    animation: none;
  }
}

.float-card .label {
  font-size: 10.5px;
  color: var(--ip-color-text-tertiary);
  font-weight: var(--ip-font-weight-medium);
  margin-bottom: 6px;
  font-family: var(--ip-font-mono);
}

.float-card .fc-row {
  display: flex;
  gap: 6px;
  align-items: center;
}

.float-card .fc-text {
  font-size: 11.5px;
  color: var(--ip-color-text-tertiary);
  max-width: 120px;
  line-height: 1.45;
}

.float-card .fc-text-strong {
  color: var(--ip-color-text-primary);
}

[data-theme='dark'] .float-card .fc-text-strong { color: var(--ip-white); }

.mini-pill {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  padding: 2px 8px;
  background: var(--ip-primary-50);
  color: var(--ip-primary-700);
  border-radius: var(--ip-radius-full);
  font-weight: var(--ip-font-weight-medium);
}

.mini-pill .dot {
  width: 6px;
  height: 6px;
  border-radius: var(--ip-radius-full);
  background: var(--ip-success-base);
}

/* ---- Center preview stage ---- */
.preview-stage {
  position: relative;
  z-index: 3;
  width: 100%;
  max-width: 480px;
  background: var(--ip-white);
  border-radius: var(--ip-radius-3xl);
  border: 1px solid var(--ip-gray-200);
  box-shadow:
    0 20px 50px rgba(20, 24, 31, 0.12),
    0 8px 20px rgba(20, 24, 31, 0.06);
  overflow: hidden;
}

[data-theme='dark'] .preview-stage {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

.preview-stage-header {
  height: 44px;
  padding: 0 var(--ip-spacing-4);
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  border-bottom: 1px solid var(--ip-gray-200);
  background: var(--ip-gray-50);
}

[data-theme='dark'] .preview-stage-header {
  background: var(--ip-gray-850);
  border-bottom-color: var(--ip-gray-700);
}

.preview-stage-header .stage-dot {
  width: 8px;
  height: 8px;
  background: var(--ip-primary-400);
  border-radius: var(--ip-radius-full);
}

.preview-stage-header .stage-name {
  font-size: 12.5px;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
}

[data-theme='dark'] .preview-stage-header .stage-name { color: var(--ip-white); }

.preview-stage-header .stage-meta {
  margin-left: auto;
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  font-family: var(--ip-font-mono);
}

.preview-stage-body {
  padding: var(--ip-spacing-5);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-4);
  min-height: 320px;
}

.msg { display: flex; gap: var(--ip-spacing-2); align-items: flex-start; }
.msg.user { flex-direction: row-reverse; }

.msg .av {
  width: 28px;
  height: 28px;
  border-radius: var(--ip-radius-md);
  display: grid;
  place-items: center;
  font-size: 12px;
  font-weight: var(--ip-font-weight-semibold);
  flex-shrink: 0;
  color: var(--ip-white);
}

.msg.user .av { background: linear-gradient(135deg, var(--ip-primary-600), var(--ip-primary-800)); }
.msg.ai .av { background: linear-gradient(135deg, var(--ip-success-base), var(--ip-success-active)); }

.msg .bubble {
  max-width: 320px;
  padding: 10px 14px;
  border-radius: 14px;
  font-size: 13px;
  line-height: 1.55;
  color: var(--ip-color-text-primary);
}

.msg.user .bubble {
  background: var(--ip-primary-500);
  color: var(--ip-white);
  border-bottom-right-radius: 4px;
}

.msg.ai .bubble {
  background: var(--ip-gray-50);
  border: 1px solid var(--ip-gray-200);
  border-bottom-left-radius: 4px;
}

[data-theme='dark'] .msg.ai .bubble {
  background: var(--ip-gray-700);
  border-color: var(--ip-gray-600);
  color: var(--ip-white);
}

.msg.ai .bubble code {
  font-family: var(--ip-font-mono);
  font-size: 11.5px;
  background: var(--ip-gray-100);
  padding: 1px 5px;
  border-radius: 4px;
  color: var(--ip-primary-700);
}

[data-theme='dark'] .msg.ai .bubble code {
  background: var(--ip-gray-600);
  color: var(--ip-primary-300);
}

.cursor {
  display: inline-block;
  width: 6px;
  height: 13px;
  background: var(--ip-primary-500);
  margin-left: 2px;
  vertical-align: -2px;
  animation: blink 1s steps(2) infinite;
}

@keyframes blink { 50% { opacity: 0; } }

@media (prefers-reduced-motion: reduce) {
  .cursor { animation: none; }
}

.msg-toolbar {
  display: flex;
  gap: 4px;
  margin-top: 6px;
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
}

.msg-toolbar span {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 2px 6px;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
}

.msg-toolbar span:hover {
  background: var(--ip-gray-100);
  color: var(--ip-color-text-primary);
}

[data-theme='dark'] .msg-toolbar span:hover {
  background: var(--ip-gray-700);
  color: var(--ip-white);
}

/* ================================================================
   Section divider
   ================================================================ */
.section-divider {
  display: flex;
  align-items: baseline;
  gap: var(--ip-spacing-3);
  margin: var(--ip-spacing-16) 0 var(--ip-spacing-8);
  padding-top: var(--ip-spacing-12);
  border-top: 1px solid var(--ip-gray-200);
}

[data-theme='dark'] .section-divider { border-top-color: var(--ip-gray-800); }

.section-divider .h2 {
  font-family: var(--ip-font-display);
  font-size: 32px;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-primary);
  letter-spacing: -0.02em;
  line-height: 1;
}

[data-theme='dark'] .section-divider .h2 { color: var(--ip-white); }

.section-divider .h2 em {
  font-style: normal;
  color: var(--ip-primary-600);
}

.section-divider .lead {
  margin-left: auto;
  max-width: 360px;
  font-size: 13.5px;
  color: var(--ip-color-text-body);
  line-height: 1.55;
  text-align: right;
}

[data-theme='dark'] .section-divider .lead { color: var(--ip-gray-300); }

/* ================================================================
   Section 1: Buttons · 不对称双栏 (1.2fr / 1fr)
   ================================================================ */
.buttons-section {
  display: grid;
  grid-template-columns: 1.2fr 1fr;
  gap: var(--ip-spacing-10);
  align-items: start;
}

.buttons-stage {
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-xl);
  padding: var(--ip-spacing-8);
  border: 1px solid var(--ip-gray-200);
  position: sticky;
  top: 80px;
}

[data-theme='dark'] .buttons-stage {
  background: var(--ip-gray-850);
  border-color: var(--ip-gray-800);
}

.buttons-stage h3 {
  font-family: var(--ip-font-display);
  font-size: 22px;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-primary);
  margin-bottom: var(--ip-spacing-2);
}

[data-theme='dark'] .buttons-stage h3 { color: var(--ip-white); }

.buttons-stage .stage-sub {
  font-size: 13px;
  color: var(--ip-color-text-tertiary);
  margin-bottom: var(--ip-spacing-6);
}

.btn-display {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--ip-spacing-10);
  background: var(--ip-white);
  border-radius: var(--ip-radius-lg);
  border: 1px solid var(--ip-gray-200);
  margin-bottom: var(--ip-spacing-5);
}

[data-theme='dark'] .btn-display {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

.btn-display .label {
  font-size: 11.5px;
  color: var(--ip-color-text-tertiary);
  margin-bottom: var(--ip-spacing-4);
  font-family: var(--ip-font-mono);
}

.btn-state-row {
  display: flex;
  gap: var(--ip-spacing-2);
  flex-wrap: wrap;
  justify-content: center;
}

.btn-pill {
  font-size: 11px;
  padding: 4px 10px;
  border-radius: var(--ip-radius-full);
  font-family: var(--ip-font-mono);
  cursor: pointer;
  border: 1px solid transparent;
  background: transparent;
  color: var(--ip-color-text-body);
  transition: all var(--ip-duration-base) var(--ip-ease-out);
}

.btn-pill.active {
  background: var(--ip-primary-500);
  color: var(--ip-white);
  border-color: var(--ip-primary-500);
}

.btn-pill:not(.active) { border-color: var(--ip-gray-200); }
.btn-pill:not(.active):hover { border-color: var(--ip-primary-300); }

.demo-force-hover { opacity: 0.85; transform: translateY(-1px); }
.demo-force-active { transform: translateY(1px); filter: brightness(0.95); }

.btn-matrix { display: grid; gap: var(--ip-spacing-4); }

.matrix-row {
  display: grid;
  grid-template-columns: 80px 1fr;
  gap: var(--ip-spacing-4);
  align-items: center;
  padding-bottom: var(--ip-spacing-4);
  border-bottom: 1px solid var(--ip-gray-200);
}

.matrix-row:last-child { border-bottom: none; padding-bottom: 0; }

.matrix-row .row-label {
  font-size: 12px;
  color: var(--ip-color-text-tertiary);
  font-weight: var(--ip-font-weight-medium);
}

.matrix-row .row-label .sub {
  display: block;
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  font-weight: var(--ip-font-weight-regular);
  margin-top: 2px;
}

.matrix-row .row-items {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-3);
  flex-wrap: wrap;
}

/* ================================================================
   Section 2: Cards · 三栏不等比 (2fr / 1fr / 1fr)
   ================================================================ */
.cards-section {
  display: grid;
  grid-template-columns: 2fr 1fr 1fr;
  gap: var(--ip-spacing-5);
}

.real-card--tinted {
  background: linear-gradient(180deg, var(--ip-primary-50) 0%, var(--ip-white) 60%) !important;
  border-color: var(--ip-primary-100) !important;
}

[data-theme='dark'] .real-card--tinted {
  background: linear-gradient(180deg, var(--ip-gray-850) 0%, var(--ip-gray-800) 60%) !important;
  border-color: var(--ip-gray-700) !important;
}

.card-meta-row {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
}

.card-meta-row .dot {
  width: 3px;
  height: 3px;
  background: var(--ip-gray-300);
  border-radius: var(--ip-radius-full);
}

.card-meta-row .meta-active {
  color: var(--ip-primary-700);
  font-weight: var(--ip-font-weight-medium);
}

.card-title {
  font-family: var(--ip-font-display);
  font-size: 22px;
  line-height: 1.15;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-primary);
  letter-spacing: -0.015em;
}

.card-title-sm { font-size: 18px; }

[data-theme='dark'] .card-title { color: var(--ip-white); }

.card-title em { font-style: normal; color: var(--ip-primary-600); }

.card-desc {
  font-size: 13px;
  line-height: 1.55;
  color: var(--ip-color-text-body);
}

.card-desc-sm { font-size: 12.5px; }

[data-theme='dark'] .card-desc { color: var(--ip-gray-300); }

/* 团队头像栈 */
.team-stack {
  display: flex;
  align-items: center;
  margin-top: auto;
  padding-top: var(--ip-spacing-3);
  border-top: 1px solid var(--ip-gray-200);
}

[data-theme='dark'] .team-stack { border-top-color: var(--ip-gray-700); }

.team-stack .stack-av {
  width: 26px;
  height: 26px;
  border-radius: var(--ip-radius-full);
  border: 2px solid var(--ip-white);
  display: grid;
  place-items: center;
  font-size: 11px;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-white);
  margin-left: -8px;
}

[data-theme='dark'] .team-stack .stack-av { border-color: var(--ip-gray-800); }

.team-stack .stack-av:first-child { margin-left: 0; }

.team-stack .stack-av.more {
  background: var(--ip-gray-100);
  color: var(--ip-color-text-body);
  font-family: var(--ip-font-mono);
  font-size: 10.5px;
}

[data-theme='dark'] .team-stack .stack-av.more {
  background: var(--ip-gray-700);
  color: var(--ip-gray-300);
}

.team-stack .team-label {
  margin-left: auto;
  font-size: 11.5px;
  color: var(--ip-color-text-tertiary);
}

.session-foot {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: auto;
}

.session-foot .stack-av {
  margin-left: 0;
}

.session-foot-label {
  font-size: 11.5px;
  color: var(--ip-color-text-tertiary);
}

/* metric card (nested selectors kept for layout) */
.mini-card {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
}

.mini-card .icon-tile {
  width: 32px;
  height: 32px;
  border-radius: var(--ip-radius-md);
  display: grid;
  place-items: center;
  color: var(--ip-white);
}

.mini-card .icon-tile.a { background: linear-gradient(135deg, var(--ip-primary-500), var(--ip-primary-700)); }
.mini-card .icon-tile.b { background: linear-gradient(135deg, var(--ip-success-base), var(--ip-success-active)); }
.mini-card .icon-tile.c { background: linear-gradient(135deg, var(--ip-warning-base), var(--ip-warning-active)); }

.mini-card .mini-title {
  font-family: var(--ip-font-display);
  font-size: 18px;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-primary);
  line-height: 1.2;
}

[data-theme='dark'] .mini-card .mini-title { color: var(--ip-white); }

.mini-card .mini-number {
  font-family: var(--ip-font-display);
  font-size: 36px;
  line-height: 1;
  color: var(--ip-color-text-primary);
}

[data-theme='dark'] .mini-card .mini-number { color: var(--ip-white); }

.mini-card .mini-meta {
  font-size: 12px;
  color: var(--ip-color-text-tertiary);
  margin-top: auto;
}

/* ================================================================
   Section 3: Message · 单栏聊天场景
   ================================================================ */
.message-section {
  max-width: var(--ip-message-max-w);
  margin: 0 auto;
}

.message-stage {
  background: var(--ip-color-bg-elevated);
  border-radius: var(--ip-radius-3xl);
  border: 1px solid var(--ip-gray-200);
  box-shadow:
    0 20px 50px rgba(20, 24, 31, 0.08),
    0 8px 20px rgba(20, 24, 31, 0.04);
  overflow: hidden;
}

[data-theme='dark'] .message-stage {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

/* ================================================================
   Section 4: Inputs · 非对称 inline 表单 (2fr / 1fr)
   ================================================================ */
.inputs-section {
  display: grid;
  grid-template-columns: 2fr 1fr;
  gap: var(--ip-spacing-8);
  align-items: start;
}

.inputs-form {
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-gray-200);
  border-radius: var(--ip-radius-xl);
  padding: var(--ip-spacing-8);
}

[data-theme='dark'] .inputs-form {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

.inputs-form h3,
.inputs-aux h3 {
  font-family: var(--ip-font-display);
  font-size: 22px;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-primary);
  margin-bottom: var(--ip-spacing-2);
}

[data-theme='dark'] .inputs-form h3,
[data-theme='dark'] .inputs-aux h3 { color: var(--ip-white); }

.inputs-form .stage-sub,
.inputs-aux .stage-sub {
  font-size: 13px;
  color: var(--ip-color-text-tertiary);
  margin-bottom: var(--ip-spacing-6);
}

.form-row {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
  margin-bottom: var(--ip-spacing-5);
}

.form-row label {
  font-size: 12px;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-body);
}

[data-theme='dark'] .form-row label { color: var(--ip-gray-300); }

.form-row--cta {
  flex-direction: row;
  justify-content: flex-end;
  gap: var(--ip-spacing-3);
  margin-top: var(--ip-spacing-6);
  padding-top: var(--ip-spacing-5);
  border-top: 1px solid var(--ip-gray-200);
  margin-bottom: 0;
}

[data-theme='dark'] .form-row--cta { border-top-color: var(--ip-gray-700); }

.inputs-aux {
  /* 不加 border */
}

.aux-label {
  display: block;
  font-size: 11px;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-tertiary);
  font-family: var(--ip-font-mono);
  margin: var(--ip-spacing-5) 0 var(--ip-spacing-3);
  letter-spacing: 0.01em;
}

.aux-label:first-of-type { margin-top: 0; }

.aux-stack {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}

.aux-row {
  display: flex;
  gap: var(--ip-spacing-2);
}

.aux-suffix-text {
  font-size: 12px;
  color: var(--ip-color-text-tertiary);
}

/* ================================================================
   Section 5: Modal · 双联对照 (1fr / 1.4fr)
   ================================================================ */
.modals-section {
  display: grid;
  grid-template-columns: 1fr 1.4fr;
  gap: var(--ip-spacing-8);
  align-items: start;
}

.modal-triggers h3,
.modal-preview h3 {
  font-family: var(--ip-font-display);
  font-size: 22px;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-primary);
  margin-bottom: var(--ip-spacing-2);
}

[data-theme='dark'] .modal-triggers h3,
[data-theme='dark'] .modal-preview h3 { color: var(--ip-white); }

.modal-triggers .stage-sub,
.modal-preview .stage-sub {
  font-size: 13px;
  color: var(--ip-color-text-tertiary);
  margin-bottom: var(--ip-spacing-6);
}

.trigger-stack {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
}

.modal-body-text {
  color: var(--ip-color-text-body);
}

.modal-form-field {
  margin-bottom: 12px;
}

.modal-anatomy {
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-gray-200);
  border-radius: var(--ip-radius-xl);
  box-shadow: var(--ip-shadow-lg);
  overflow: hidden;
  max-width: 560px;
  margin-bottom: var(--ip-spacing-6);
}

[data-theme='dark'] .modal-anatomy {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

.anatomy-row {
  padding: var(--ip-spacing-4) var(--ip-spacing-5);
  display: flex;
}

.anatomy-row--header { justify-content: space-between; align-items: center; }
.anatomy-row--footer { justify-content: flex-end; gap: 8px; }

.anatomy-label {
  font-size: 15px;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

[data-theme='dark'] .anatomy-label { color: var(--ip-white); }

.anatomy-close {
  font-size: 20px;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  line-height: 1;
}

.anatomy-divider {
  border: 0;
  border-top: 1px solid var(--ip-gray-200);
  margin: 0;
}

[data-theme='dark'] .anatomy-divider { border-top-color: var(--ip-gray-700); }

.anatomy-body {
  padding: var(--ip-spacing-5);
  font-size: 13px;
  color: var(--ip-color-text-body);
  line-height: 1.6;
}

.anatomy-body-meta {
  color: var(--ip-color-text-tertiary);
  margin-top: 12px;
}

[data-theme='dark'] .anatomy-body { color: var(--ip-gray-300); }

.anatomy-list {
  list-style: none;
  padding: 0;
  display: grid;
  gap: var(--ip-spacing-2);
}

.anatomy-list li {
  font-size: 13px;
  color: var(--ip-color-text-body);
  padding-left: var(--ip-spacing-4);
  position: relative;
}

.anatomy-list li::before {
  content: "·";
  position: absolute;
  left: var(--ip-spacing-2);
  color: var(--ip-primary-500);
  font-weight: bold;
}

.anatomy-list strong {
  color: var(--ip-color-text-primary);
  font-weight: var(--ip-font-weight-semibold);
  margin-right: 6px;
}

[data-theme='dark'] .anatomy-list strong { color: var(--ip-white); }

/* ================================================================
   Section 6: Toast · 横向时间线
   ================================================================ */
.toast-timeline {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: var(--ip-spacing-4);
  margin-bottom: var(--ip-spacing-8);
}

.toast-step {
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-gray-200);
  border-radius: var(--ip-radius-lg);
  padding: var(--ip-spacing-5);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
  position: relative;
}

[data-theme='dark'] .toast-step {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

.toast-step-marker {
  width: 8px;
  height: 8px;
  border-radius: var(--ip-radius-full);
  position: absolute;
  top: var(--ip-spacing-5);
  right: var(--ip-spacing-5);
}

.toast-step-marker.success { background: var(--ip-success-base); }
.toast-step-marker.info { background: var(--ip-info-base); }
.toast-step-marker.warning { background: var(--ip-warning-base); }
.toast-step-marker.error { background: var(--ip-danger-base); }

.toast-step-content h4 {
  font-family: var(--ip-font-display);
  font-size: 18px;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-primary);
  margin-bottom: 4px;
}

[data-theme='dark'] .toast-step-content h4 { color: var(--ip-white); }

.toast-step-content p {
  font-size: 12px;
  color: var(--ip-color-text-tertiary);
  line-height: 1.5;
}

.toast-step-content code {
  font-family: var(--ip-font-mono);
  font-size: 11px;
  color: var(--ip-primary-700);
  background: var(--ip-primary-50);
  padding: 2px 6px;
  border-radius: var(--ip-radius-sm);
  display: inline-block;
  margin-top: 4px;
}

.toast-merge {
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-lg);
  padding: var(--ip-spacing-6);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
  align-items: flex-start;
}

[data-theme='dark'] .toast-merge {
  background: var(--ip-gray-850);
}

.toast-merge h4 {
  font-family: var(--ip-font-display);
  font-size: 18px;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-primary);
}

[data-theme='dark'] .toast-merge h4 { color: var(--ip-white); }

.toast-merge p {
  font-size: 13px;
  color: var(--ip-color-text-body);
}

/* ================================================================
   Section 7: Avatar · 3 行布局
   ================================================================ */
.avatar-section {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-6);
}

.avatar-size-strip,
.avatar-type-row,
.avatar-shape-row {
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-gray-200);
  border-radius: var(--ip-radius-lg);
  padding: var(--ip-spacing-5);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-4);
}

[data-theme='dark'] .avatar-size-strip,
[data-theme='dark'] .avatar-type-row,
[data-theme='dark'] .avatar-shape-row {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

.strip-label {
  font-size: 11px;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-tertiary);
  font-family: var(--ip-font-mono);
  letter-spacing: 0.01em;
}

.strip-meta {
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  font-family: var(--ip-font-mono);
}

.avatar-type {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  text-align: center;
  min-width: 80px;
}

.avatar-type .type-name {
  font-size: 12px;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
}

[data-theme='dark'] .avatar-type .type-name { color: var(--ip-white); }

.avatar-type .type-desc {
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
}

/* ================================================================
   Section 8: Select · 带描述的选项
   ================================================================ */
.select-section {
  max-width: 720px;
  margin: 0 auto;
}

.select-stage {
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-gray-200);
  border-radius: var(--ip-radius-xl);
  padding: var(--ip-spacing-8);
}

[data-theme='dark'] .select-stage {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

.select-stage h3 {
  font-family: var(--ip-font-display);
  font-size: 22px;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-primary);
  margin-bottom: var(--ip-spacing-2);
}

[data-theme='dark'] .select-stage h3 { color: var(--ip-white); }

.select-stage .stage-sub {
  font-size: 13px;
  color: var(--ip-color-text-tertiary);
  margin-bottom: var(--ip-spacing-6);
}

.select-error-trigger {
  align-self: flex-start;
  margin-top: 8px;
}

/* ================================================================
   Section 9: Empty · 2x2 grid 四种差异化空状态
   ================================================================ */
.empty-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--ip-spacing-5);
}

.empty-steps {
  list-style: none;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
  margin: var(--ip-spacing-2) 0;
}

.empty-steps li {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-3);
  font-size: 13px;
  color: var(--ip-color-text-body);
}

[data-theme='dark'] .empty-steps li { color: var(--ip-gray-300); }

.step-num {
  width: 24px;
  height: 24px;
  border-radius: var(--ip-radius-full);
  background: var(--ip-primary-100);
  color: var(--ip-primary-700);
  display: grid;
  place-items: center;
  font-size: 12px;
  font-weight: var(--ip-font-weight-semibold);
  font-family: var(--ip-font-mono);
}

/* ================================================================
   Section 10: Dropdown · demo grid
   ================================================================ */
.dropdown-section {
  max-width: 720px;
  margin: 0 auto;
}

.dropdown-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--ip-spacing-5);
}

.dropdown-demo-block {
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-gray-200);
  border-radius: var(--ip-radius-lg);
  padding: var(--ip-spacing-5);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
  align-items: flex-start;
}

[data-theme='dark'] .dropdown-demo-block {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

.dropdown-demo-block h4 {
  font-size: 14px;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
}

[data-theme='dark'] .dropdown-demo-block h4 { color: var(--ip-white); }

/* ================================================================
   Section 11: Popconfirm
   ================================================================ */
.popconfirm-section {
  max-width: var(--ip-message-max-w);
  margin: 0 auto;
}

.popconfirm-stage {
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-gray-200);
  border-radius: var(--ip-radius-xl);
  padding: var(--ip-spacing-8);
}

[data-theme='dark'] .popconfirm-stage {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

.popconfirm-stage h3 {
  font-family: var(--ip-font-display);
  font-size: 22px;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-primary);
  margin-bottom: var(--ip-spacing-2);
}

[data-theme='dark'] .popconfirm-stage h3 { color: var(--ip-white); }

.popconfirm-stage .stage-sub {
  font-size: 13px;
  color: var(--ip-color-text-tertiary);
  margin-bottom: var(--ip-spacing-6);
}

.pop-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--ip-spacing-4);
}

.pop-item {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-3);
  padding: var(--ip-spacing-4);
  background: var(--ip-gray-50);
  border-radius: var(--ip-radius-md);
}

[data-theme='dark'] .pop-item { background: var(--ip-gray-850); }

.pop-item--wide {
  grid-column: 1 / -1;
  flex-direction: column;
  align-items: flex-start;
}

.pop-item--wide h4 {
  font-size: 13px;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-body);
  margin-bottom: var(--ip-spacing-2);
}

[data-theme='dark'] .pop-item--wide h4 { color: var(--ip-gray-300); }

.pop-trigger {
  font-size: 12px;
  color: var(--ip-color-text-tertiary);
  padding: 4px 8px;
  background: var(--ip-white);
  border: 1px dashed var(--ip-gray-300);
  border-radius: var(--ip-radius-sm);
}

[data-theme='dark'] .pop-trigger {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

/* ================================================================
   Section 12: Flex · 轴线矩阵
   ================================================================ */
.flex-matrix {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--ip-spacing-5);
}

.matrix-block {
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-gray-200);
  border-radius: var(--ip-radius-lg);
  padding: var(--ip-spacing-5);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
}

[data-theme='dark'] .matrix-block {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

.matrix-block h4 {
  font-size: 12px;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-tertiary);
  font-family: var(--ip-font-mono);
  margin-bottom: var(--ip-spacing-2);
}

.chip {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 6px 10px;
  background: var(--ip-primary-50);
  color: var(--ip-primary-700);
  border-radius: var(--ip-radius-md);
  font-size: 12px;
  font-weight: var(--ip-font-weight-medium);
}

.chip-tall { height: 60px; }

.gap-row {
  display: grid;
  grid-template-columns: 100px 1fr;
  align-items: center;
  gap: var(--ip-spacing-3);
}

.gap-row code {
  font-family: var(--ip-font-mono);
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
}

.justify-track {
  background: var(--ip-gray-50);
  padding: var(--ip-spacing-2);
  border-radius: var(--ip-radius-sm);
  width: 100%;
}

[data-theme='dark'] .justify-track { background: var(--ip-gray-850); }

.align-track {
  background: var(--ip-gray-50);
  padding: var(--ip-spacing-2);
  border-radius: var(--ip-radius-sm);
  height: 80px;
  display: flex;
  align-items: center;
}

[data-theme='dark'] .align-track { background: var(--ip-gray-850); }

.align-row { width: 100%; }

/* ================================================================
   Section 13: Container · 宽度阶梯
   ================================================================ */
.container-section {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--ip-spacing-8);
  align-items: start;
}

.container-stage,
.container-extras {
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-gray-200);
  border-radius: var(--ip-radius-xl);
  padding: var(--ip-spacing-6);
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
}

[data-theme='dark'] .container-stage,
[data-theme='dark'] .container-extras {
  background: var(--ip-gray-800);
  border-color: var(--ip-gray-700);
}

.container-stage h3,
.container-extras h4 {
  font-family: var(--ip-font-display);
  font-size: 20px;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-primary);
}

[data-theme='dark'] .container-stage h3,
[data-theme='dark'] .container-extras h4 { color: var(--ip-white); }

.container-stage .stage-sub {
  font-size: 13px;
  color: var(--ip-color-text-tertiary);
  margin-bottom: var(--ip-spacing-3);
}

.container-frame {
  background: var(--ip-primary-50);
  border: 1px dashed var(--ip-primary-300);
  border-radius: var(--ip-radius-md);
  padding: var(--ip-spacing-3);
  font-size: 12px;
  color: var(--ip-primary-700);
  font-family: var(--ip-font-mono);
  text-align: center;
}

.container-frame--small { font-size: 11px; padding: 6px; }
.container-frame--fluid {
  background: var(--ip-warning-bg);
  border-color: var(--ip-warning-base);
  color: var(--ip-warning-base);
}

.extra-pair {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--ip-spacing-3);
  align-items: center;
}

.extra-context code {
  font-family: var(--ip-font-mono);
  font-size: 11px;
  color: var(--ip-primary-700);
  background: var(--ip-primary-50);
  padding: 2px 6px;
  border-radius: var(--ip-radius-sm);
  display: inline-block;
  margin-bottom: 4px;
}

.extra-context p {
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  line-height: 1.4;
}

/* ================================================================
   Footer
   ================================================================ */
.footer {
  margin-top: var(--ip-spacing-24);
  padding: var(--ip-spacing-10) 0 var(--ip-spacing-12);
  border-top: 1px solid var(--ip-gray-200);
  display: grid;
  grid-template-columns: 2fr 1fr 1fr 1fr;
  gap: var(--ip-spacing-8);
}

[data-theme='dark'] .footer {
  border-top-color: var(--ip-gray-800);
}

.footer-brand {
  font-family: var(--ip-font-display);
  font-size: 22px;
  color: var(--ip-color-text-primary);
  margin-bottom: var(--ip-spacing-2);
}

[data-theme='dark'] .footer-brand { color: var(--ip-white); }

.footer-tag {
  font-size: 13px;
  color: var(--ip-color-text-tertiary);
  max-width: 320px;
  line-height: 1.55;
}

.footer-col h4 {
  font-size: 12px;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-tertiary);
  margin-bottom: var(--ip-spacing-3);
}

.footer-col a {
  display: block;
  font-size: 13px;
  color: var(--ip-color-text-body);
  text-decoration: none;
  padding: 4px 0;
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}

[data-theme='dark'] .footer-col a { color: var(--ip-gray-300); }

.footer-col a:hover { color: var(--ip-primary-700); }

.footer-snow {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: var(--ip-spacing-4);
  color: var(--ip-primary-400);
}

.footer-snow svg { width: 12px; height: 12px; }

.footer-snow-text {
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  margin-left: 4px;
}

/* ================================================================
   Section reveal (single fade-in, no stagger)
   ================================================================ */
.reveal-init {
  opacity: 0;
  transform: translateY(12px);
  transition:
    opacity var(--ip-duration-page) var(--ip-ease-out),
    transform var(--ip-duration-page) var(--ip-ease-out);
}

.reveal-init.is-revealed {
  opacity: 1;
  transform: translateY(0);
}

@media (prefers-reduced-motion: reduce) {
  .reveal-init {
    opacity: 1;
    transform: none;
    transition: none;
  }
}

/* ================================================================
   Responsive
   ================================================================ */
@media (max-width: 1023px) {
  .hero {
    grid-template-columns: 1fr;
    gap: var(--ip-spacing-8);
  }

  .hero-visual { height: 400px; }

  .buttons-section,
  .inputs-section,
  .modals-section,
  .empty-grid,
  .flex-matrix,
  .container-section {
    grid-template-columns: 1fr;
  }

  .cards-section {
    grid-template-columns: 1fr 1fr;
  }

  .toast-timeline {
    grid-template-columns: 1fr 1fr;
  }

  .pop-grid {
    grid-template-columns: 1fr;
  }

  .footer {
    grid-template-columns: 1fr 1fr;
    gap: var(--ip-spacing-6);
  }

  .dropdown-section,
  .select-section {
    max-width: none;
  }
}

@media (max-width: 767px) {
  .page { padding: 0 var(--ip-spacing-5); }
  .page.hero { padding: var(--ip-spacing-12) var(--ip-spacing-5) var(--ip-spacing-10); }
  .nav-inner { gap: var(--ip-spacing-3); }
  .nav-links { display: none; }
  .hero h1 { font-size: clamp(2rem, 8vw, 2.8rem); }
  .hero-visual { height: 360px; }
  .float-card.f1 { left: 0; }
  .float-card.f2 { left: -10px; }
  .float-card.f3 { right: 0; }
  .float-card.f4 { right: 0; }

  .cards-section,
  .toast-timeline,
  .footer {
    grid-template-columns: 1fr;
  }

  .hero-meta {
    flex-direction: column;
    align-items: flex-start;
    gap: var(--ip-spacing-2);
  }

  .hero-meta .dot { display: none; }

  .section-divider {
    flex-direction: column;
    align-items: flex-start;
    gap: var(--ip-spacing-2);
  }

  .section-divider .lead {
    margin-left: 0;
    text-align: left;
    max-width: none;
  }

  .popconfirm-section,
  .message-section {
    max-width: none;
  }
}
</style>


<!-- Unscoped: smooth scroll must apply to html/body -->
<style>
html,
body {
  scroll-behavior: smooth;
}
</style>
