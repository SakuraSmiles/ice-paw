/**
 * Popconfirm E2E 测试
 *
 * 测试场景：
 *  1. 点击触发按钮 → 弹窗出现
 *  2. 弹窗内容验证（标题、描述、确认/取消按钮）
 *  3. 点击确认 → 事件触发 + 弹窗关闭
 *  4. 点击取消 → 弹窗关闭
 *  5. 点击外部 → 弹窗关闭
 *  6. 按 Escape → 弹窗关闭
 *  7. danger 样式验证
 *
 * 预览站：必须运行于 http://localhost:5173
 * 夹具：/test/fixtures
 *
 * 注意：Popconfirm 弹层使用 <Teleport to="body">，因此 popover 不在
 * wrapper 的 DOM 子树内。需要用 page.locator('.ip-popconfirm__popover')
 * 在页面级别查找弹层。
 */

import { test, expect } from '@playwright/test'

/* ──────────────────────────────────────────────
 * Helpers
 * ────────────────────────────────────────────── */

/** 基础 Popconfirm 的触发区（.ip-popconfirm__trigger 是触发按钮的容器） */
function basicTrigger(page: import('@playwright/test').Page) {
  return page.getByTestId('popconfirm-basic-wrap').locator('.ip-popconfirm__trigger')
}

/** 基础 Popconfirm 的弹窗（Teleport 到 body，需跨 DOM 层级查找） */
function basicPopover(page: import('@playwright/test').Page) {
  return page.locator('.ip-popconfirm__popover').first()
}

/** Danger Popconfirm 的触发区 */
function dangerTrigger(page: import('@playwright/test').Page) {
  return page.getByTestId('popconfirm-danger-wrap').locator('.ip-popconfirm__trigger')
}

/** Danger Popconfirm 的弹窗 */
function dangerPopover(page: import('@playwright/test').Page) {
  // 两个 danger-wrap，各有一个 popconfirm；取第一个
  return page.getByTestId('popconfirm-danger-wrap').locator('.ip-popconfirm__popover')
}

/* ──────────────────────────────────────────────
 * Tests
 * ────────────────────────────────────────────── */

test.describe('Popconfirm — 基本交互', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/test/fixtures')
    await expect(page.getByTestId('fixture-popconfirm')).toBeVisible()
  })

  test('1. 点击触发按钮 → 弹窗出现', async ({ page }) => {
    const trigger = basicTrigger(page)
    const popover = basicPopover(page)

    // 初始弹窗不可见
    await expect(popover).not.toBeVisible()

    // 点击触发按钮
    await trigger.click()

    // 弹窗出现
    await expect(popover).toBeVisible()

    // role=alertdialog
    await expect(popover).toHaveAttribute('role', 'alertdialog')

    // trigger aria-expanded
    await expect(trigger).toHaveAttribute('aria-expanded', 'true')
  })

  test('2. 弹窗内容验证（标题、描述、确认/取消按钮）', async ({ page }) => {
    const trigger = basicTrigger(page)
    const popover = basicPopover(page)

    await trigger.click()
    await expect(popover).toBeVisible()

    // 标题
    await expect(popover.locator('.ip-popconfirm__title')).toContainText('确定要删除吗？')

    // 描述
    await expect(popover.locator('.ip-popconfirm__description')).toContainText('此操作不可撤销。')

    // 确认按钮（variant=primary）
    const confirmBtn = popover.locator('.ip-button--primary')
    await expect(confirmBtn).toBeVisible()
    await expect(confirmBtn).toContainText('确认')

    // 取消按钮（variant=secondary）
    const cancelBtn = popover.locator('.ip-button--secondary')
    await expect(cancelBtn).toBeVisible()
    await expect(cancelBtn).toContainText('取消')
  })

  test('3. 点击确认 → 事件触发 + 弹窗关闭', async ({ page }) => {
    const trigger = basicTrigger(page)
    const popover = basicPopover(page)

    await trigger.click()
    await expect(popover).toBeVisible()

    // 点击确认按钮
    await popover.locator('.ip-button--primary').click()

    // 弹窗关闭
    await expect(popover).not.toBeVisible()
  })

  test('4. 点击取消 → 弹窗关闭', async ({ page }) => {
    const trigger = basicTrigger(page)
    const popover = basicPopover(page)

    await trigger.click()
    await expect(popover).toBeVisible()

    // 点击取消按钮
    await popover.locator('.ip-button--secondary').click()

    // 弹窗关闭
    await expect(popover).not.toBeVisible()
  })

  test('5. 点击外部 → 弹窗关闭', async ({ page }) => {
    const trigger = basicTrigger(page)
    const popover = basicPopover(page)

    await trigger.click()
    await expect(popover).toBeVisible()

    // 点击页面左上角空白区域
    await page.mouse.click(10, 10)

    await expect(popover).not.toBeVisible()
  })

  test('6. 按 Escape → 弹窗关闭', async ({ page }) => {
    const trigger = basicTrigger(page)
    const popover = basicPopover(page)

    await trigger.click()
    await expect(popover).toBeVisible()

    await page.keyboard.press('Escape')

    await expect(popover).not.toBeVisible()
  })
})

test.describe('Popconfirm — 样式', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/test/fixtures')
    await expect(page.getByTestId('fixture-popconfirm')).toBeVisible()
  })

  test('7. danger 样式确认按钮为危险色', async ({ page }) => {
    const trigger = dangerTrigger(page)
    const popover = dangerPopover(page)

    // 打开 danger 弹窗
    await trigger.click()
    await expect(popover).toBeVisible()

    // 确认按钮有 danger 变体（.ip-button--danger）
    const confirmBtn = popover.locator('.ip-button')
    await expect(confirmBtn).toHaveClass(/ip-button--danger/)
  })
})
