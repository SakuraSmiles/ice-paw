/**
 * DropdownMenu E2E 测试
 *
 * 测试场景：
 *  1. 点击触发器 → 菜单展开
 *  2. hover 菜单项 → 高亮
 *  3. 点击菜单项 → 菜单关闭
 *  4. 分隔线验证
 *  5. 点击外部 → 关闭
 *  6. 按 Escape → 关闭
 *
 * 预览站：必须运行于 http://localhost:5173
 * 夹具：/test/fixtures
 */

import { test, expect } from '@playwright/test'

/* ──────────────────────────────────────────────
 * Tests
 * ────────────────────────────────────────────── */

test.describe('DropdownMenu — 基本交互', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/test/fixtures')
    await expect(page.getByTestId('fixture-dropdown')).toBeVisible()
  })

  test('1. 点击触发器 → 菜单展开', async ({ page }) => {
    const wrap = page.getByTestId('dropdown-divider-wrap')
    const trigger = wrap.locator('.ip-dropdown__trigger')
    const popover = wrap.locator('.ip-dropdown__popover')

    // 初始关闭
    await expect(popover).not.toBeVisible()

    // 点击触发器
    await trigger.click()

    // 菜单展开
    await expect(popover).toBeVisible()

    // role=menu
    await expect(popover).toHaveAttribute('role', 'menu')

    // trigger aria-expanded
    await expect(trigger).toHaveAttribute('aria-expanded', 'true')

    // 有菜单项
    const items = popover.locator('.ip-dropdown__item')
    await expect(items).toHaveCount(7)
  })

  test('2. hover 菜单项 → 高亮', async ({ page }) => {
    const wrap = page.getByTestId('dropdown-divider-wrap')
    await wrap.locator('.ip-dropdown__trigger').click()

    const popover = wrap.locator('.ip-dropdown__popover')
    await expect(popover).toBeVisible()

    // hover 第一个菜单项
    const firstItem = popover.locator('.ip-dropdown__item').first()
    await firstItem.hover()

    // 高亮 class 出现
    await expect(firstItem).toHaveClass(/ip-dropdown__item--focused/)
  })

  test('3. 点击菜单项 → 菜单关闭', async ({ page }) => {
    const wrap = page.getByTestId('dropdown-divider-wrap')
    await wrap.locator('.ip-dropdown__trigger').click()

    const popover = wrap.locator('.ip-dropdown__popover')
    await expect(popover).toBeVisible()

    // 点击第一个菜单项 "复制"
    await popover.locator('.ip-dropdown__item').first().click()

    // 菜单关闭
    await expect(popover).not.toBeVisible()
  })

  test('4. 分隔线验证', async ({ page }) => {
    const wrap = page.getByTestId('dropdown-divider-wrap')
    await wrap.locator('.ip-dropdown__trigger').click()

    const popover = wrap.locator('.ip-dropdown__popover')
    await expect(popover).toBeVisible()

    // 分隔线存在且有 role=separator
    const dividers = popover.locator('.ip-dropdown__divider')
    const count = await dividers.count()
    expect(count).toBeGreaterThanOrEqual(1)

    // 第一个分隔线
    const firstDivider = dividers.first()
    await expect(firstDivider).toHaveAttribute('role', 'separator')
  })

  test('5. 点击外部 → 关闭', async ({ page }) => {
    const wrap = page.getByTestId('dropdown-divider-wrap')
    await wrap.locator('.ip-dropdown__trigger').click()

    const popover = wrap.locator('.ip-dropdown__popover')
    await expect(popover).toBeVisible()

    // 点击页面空白区域
    await page.click('body', { position: { x: 10, y: 10 } })

    await expect(popover).not.toBeVisible()
  })

  test('6. 按 Escape → 关闭', async ({ page }) => {
    const wrap = page.getByTestId('dropdown-divider-wrap')
    await wrap.locator('.ip-dropdown__trigger').click()

    const popover = wrap.locator('.ip-dropdown__popover')
    await expect(popover).toBeVisible()

    await page.keyboard.press('Escape')

    await expect(popover).not.toBeVisible()
  })
})
