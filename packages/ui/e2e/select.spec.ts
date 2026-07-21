/**
 * Select E2E 测试
 *
 * 测试场景：
 *  1. 点击 trigger 验证下拉面板展开
 *  2. 点击选项验证选中并面板关闭
 *  3. 再次点击 trigger 验证面板关闭
 *  4. 点击外部验证面板关闭
 *  5. 按 Escape 验证面板关闭
 *  6. 选中后验证 clearable 按钮出现
 *  7. 点击 clear 验证清空
 *  8. disabled 状态验证不可展开
 *  9. error 状态验证红色边框
 *
 * 预览站：必须运行于 http://localhost:5173
 * 夹具：/test/fixtures
 *
 * 注意：Select 弹层使用 <Teleport to="body">，因此 popover/listbox 不在
 * 组件 wrapper 的 DOM 子树内。需要用 page.locator('.ip-select__popover')
 * 而非 wrap.getByRole('listbox') 来定位弹层。
 */

import { test, expect } from '@playwright/test'

/* ──────────────────────────────────────────────
 * Helpers
 * ────────────────────────────────────────────── */

/** 基础 Select 的 trigger */
function basicTrigger(page: import('@playwright/test').Page) {
  return page.getByTestId('select-basic-wrap').locator('[role="combobox"]')
}

/** 基础 Select 的 popover（Teleport 到 body，需跨 DOM 层级查找） */
function basicPopover(page: import('@playwright/test').Page) {
  return page.locator('.ip-select__popover').first()
}

/** 基础 Select 内指定选项 */
function basicOption(page: import('@playwright/test').Page, label: string) {
  return page.locator('.ip-select__option', { hasText: label }).first()
}

/* ──────────────────────────────────────────────
 * Tests
 * ────────────────────────────────────────────── */

test.describe('Select — 基本交互', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/test/fixtures')
    // 确认 select section 加载
    await expect(page.getByTestId('fixture-select')).toBeVisible()
  })

  test('1. 点击 trigger 展开下拉面板', async ({ page }) => {
    const trigger = basicTrigger(page)
    await trigger.click()

    // 面板展开（popover Teleport 到 body，需 page 级查找）
    const popover = basicPopover(page)
    await expect(popover).toBeVisible()
    await expect(popover).toHaveAttribute('role', 'listbox')

    // aria-expanded 状态
    await expect(trigger).toHaveAttribute('aria-expanded', 'true')
  })

  test('2. 点击选项 → 选中并关闭面板', async ({ page }) => {
    const trigger = basicTrigger(page)
    const popover = basicPopover(page)

    // 打开面板
    await trigger.click()
    await expect(popover).toBeVisible()

    // 点击选项 "友好"
    await basicOption(page, '友好').click()

    // 面板关闭
    await expect(popover).not.toBeVisible()
    await expect(trigger).toHaveAttribute('aria-expanded', 'false')

    // trigger 显示选中值
    await expect(trigger).toContainText('友好')
  })

  test('3. 再次点击 trigger → 关闭面板', async ({ page }) => {
    const trigger = basicTrigger(page)
    const popover = basicPopover(page)

    // 打开
    await trigger.click()
    await expect(popover).toBeVisible()

    // 再点 trigger，关闭
    await trigger.click()
    await expect(popover).not.toBeVisible()
  })

  test('4. 点击外部 → 关闭面板', async ({ page }) => {
    const trigger = basicTrigger(page)
    const popover = basicPopover(page)

    await trigger.click()
    await expect(popover).toBeVisible()

    // 点击页面左上角空白区域（fixture-select section 之外）
    await page.mouse.click(10, 10)
    await expect(popover).not.toBeVisible()
  })

  test('5. 按 Escape → 关闭面板', async ({ page }) => {
    const trigger = basicTrigger(page)
    const popover = basicPopover(page)

    await trigger.click()
    await expect(popover).toBeVisible()

    // 按 Escape
    await page.keyboard.press('Escape')
    await expect(popover).not.toBeVisible()
  })
})

test.describe('Select — Clearable', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/test/fixtures')
    await expect(page.getByTestId('fixture-select')).toBeVisible()
  })

  test('6. 选中后 clear 按钮可见', async ({ page }) => {
    // clearable select 预选了 opt-b，对应 "Claude 3.5"
    const clearableWrap = page.getByTestId('select-clearable-wrap')
    const trigger = clearableWrap.getByRole('combobox')

    // hover 触发 clear 按钮出现
    await trigger.hover()
    const clearBtn = clearableWrap.getByRole('button', { name: '清空' })
    await expect(clearBtn).toBeVisible()
  })

  test('7. 点击 clear → 清空选中值', async ({ page }) => {
    const clearableWrap = page.getByTestId('select-clearable-wrap')
    const trigger = clearableWrap.getByRole('combobox')

    // hover 显示 clear 按钮
    await trigger.hover()
    const clearBtn = clearableWrap.getByRole('button', { name: '清空' })
    await clearBtn.click()

    // trigger 回到 placeholder
    await expect(trigger).toContainText('选择一个模型')
  })
})

test.describe('Select — 状态', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/test/fixtures')
    await expect(page.getByTestId('fixture-select')).toBeVisible()
  })

  test('8. disabled 状态不可展开', async ({ page }) => {
    const disabledWrap = page.getByTestId('select-disabled-wrap')
    const trigger = disabledWrap.getByRole('combobox')

    // disabled 属性存在
    await expect(trigger).toHaveAttribute('aria-disabled', 'true')

    // 点击不展开（listbox 不存在）
    await trigger.click()
    await expect(disabledWrap.getByRole('listbox')).not.toBeVisible()
  })

  test('9. error 状态红色边框', async ({ page }) => {
    const errorWrap = page.getByTestId('select-error-wrap')
    const root = errorWrap.locator('.ip-select')

    // 初始无 error class
    await expect(root).not.toHaveClass(/ip-select--error/)

    // 触发错误
    await errorWrap.getByTestId('select-error-trigger').click()

    // error class 出现
    await expect(root).toHaveClass(/ip-select--error/)

    // trigger 有错误边框色
    const trigger = errorWrap.getByRole('combobox')
    await expect(trigger).toHaveCSS('border-color', 'rgb(239, 68, 68)')
  })
})
