import { defineConfig, devices } from '@playwright/test'

/**
 * Playwright E2E 配置 — IcePaw UI
 *
 * 预览站必须先运行：
 *   cd /mnt/d/workspace/ice-paw/packages/ui && pnpm dev
 *
 * 运行测试：
 *   cd /mnt/d/workspace/ice-paw/packages/ui && pnpm test:e2e
 *
 * 调试模式：
 *   cd /mnt/d/workspace/ice-paw/packages/ui && pnpm test:e2e --ui
 *   cd /mnt/d/workspace/ice-paw/packages/ui && pnpm test:e2e --headed
 */
export default defineConfig({
  testDir: './e2e',

  /* 预览站地址 */
  baseURL: 'http://localhost:5173',

  /* 全量超时 */
  timeout: 30_000,

  /* 预期失败用例（开发阶段允许失败，修复后移除） */
  // ignoreSnapshots: true,

  /* 禁止并发，避免同一页面状态冲突 */
  fullyParallel: false,

  /* 失败重跑 */
  retries: process.env.CI ? 1 : 0,

  /* reporter */
  reporter: [
    ['list'],
    ['html', { open: 'never' }],
  ],

  use: {
    /* 复用 baseURL */
    baseURL: 'http://localhost:5173',

    /* 截图 / trace */
    screenshot: 'only-on-failure',
    trace: 'on-first-retry',

    /* 导航超时 */
    actionTimeout: 10_000,

    /* 视口 */
    viewport: { width: 1280, height: 720 },

    /* 忽略 HTTPS 错误（dev 环境） */
    ignoreHTTPSErrors: true,
  },

  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        /* 使用系统 Chrome 避免下载 */
        channel: 'chrome',
        launchOptions: {
          executablePath: '/usr/bin/google-chrome',
          args: [
            '--no-sandbox',
            '--disable-dev-shm-usage',
          ],
        },
      },
    },
  ],

  /* Web 服务器 — 自行启动预览站时不需要
     若 CI 环境没有运行 dev server，可取消注释：
  webServer: {
    command: 'pnpm dev',
    url: 'http://localhost:5173',
    reuseExistingServer: true,
    timeout: 30_000,
  },
  */
})
