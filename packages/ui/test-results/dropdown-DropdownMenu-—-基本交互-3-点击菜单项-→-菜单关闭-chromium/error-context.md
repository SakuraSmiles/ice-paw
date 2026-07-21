# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: dropdown.spec.ts >> DropdownMenu — 基本交互 >> 3. 点击菜单项 → 菜单关闭
- Location: e2e/dropdown.spec.ts:85:3

# Error details

```
Error: expect(locator).not.toBeVisible() failed

Locator:  locator('.ip-dropdown__popover').first()
Expected: not visible
Received: visible
Timeout:  5000ms

Call log:
  - Expect "not toBeVisible" with timeout 5000ms
  - waiting for locator('.ip-dropdown__popover').first()
    14 × locator resolved to <div role="menu" aria-label="操作" data-v-0d900266="" id="ip-dropdown-v-6-menu" class="ip-dropdown__popover">…</div>
       - unexpected value "visible"

```

```yaml
- menu "操作":
  - text: 编辑
  - menuitem "复制 ⌘C"
  - menuitem "粘贴 ⌘V"
  - menuitem "剪切 ⌘X"
  - separator
  - text: 导出
  - menuitem "导出为 PDF"
  - menuitem "导出为 CSV"
  - separator
  - menuitem "删除 ⌫"
```

# Test source

```ts
  1   | /**
  2   |  * DropdownMenu E2E 测试
  3   |  *
  4   |  * 测试场景：
  5   |  *  1. 点击触发器 → 菜单展开
  6   |  *  2. hover 菜单项 → 高亮
  7   |  *  3. 点击菜单项 → 菜单关闭
  8   |  *  4. 分隔线验证
  9   |  *  5. 点击外部 → 关闭
  10  |  *  6. 按 Escape → 关闭
  11  |  *
  12  |  * 预览站：必须运行于 http://localhost:5173
  13  |  * 夹具：/test/fixtures
  14  |  *
  15  |  * 注意：DropdownMenu 弹层使用 <Teleport to="body">，因此 popover 不在
  16  |  * wrapper 的 DOM 子树内。需要用 page.locator('.ip-dropdown__popover')
  17  |  * 在页面级别查找弹层。
  18  |  */
  19  | 
  20  | import { test, expect } from '@playwright/test'
  21  | 
  22  | /* ──────────────────────────────────────────────
  23  |  * Helpers
  24  |  * ────────────────────────────────────────────── */
  25  | 
  26  | /** Dropdown 的触发器 */
  27  | function ddTrigger(page: import('@playwright/test').Page) {
  28  |   return page.getByTestId('dropdown-divider-wrap').locator('.ip-dropdown__trigger')
  29  | }
  30  | 
  31  | /** Dropdown 的菜单（Teleport 到 body，需跨 DOM 层级查找） */
  32  | function ddPopover(page: import('@playwright/test').Page) {
  33  |   return page.locator('.ip-dropdown__popover').first()
  34  | }
  35  | 
  36  | /* ──────────────────────────────────────────────
  37  |  * Tests
  38  |  * ────────────────────────────────────────────── */
  39  | 
  40  | test.describe('DropdownMenu — 基本交互', () => {
  41  |   test.beforeEach(async ({ page }) => {
  42  |     await page.goto('/test/fixtures')
  43  |     await expect(page.getByTestId('fixture-dropdown')).toBeVisible()
  44  |   })
  45  | 
  46  |   test('1. 点击触发器 → 菜单展开', async ({ page }) => {
  47  |     const trigger = ddTrigger(page)
  48  |     const popover = ddPopover(page)
  49  | 
  50  |     // 初始关闭
  51  |     await expect(popover).not.toBeVisible()
  52  | 
  53  |     // 点击触发器
  54  |     await trigger.click()
  55  | 
  56  |     // 菜单展开
  57  |     await expect(popover).toBeVisible()
  58  | 
  59  |     // role=menu
  60  |     await expect(popover).toHaveAttribute('role', 'menu')
  61  | 
  62  |     // trigger aria-expanded
  63  |     await expect(trigger).toHaveAttribute('aria-expanded', 'true')
  64  | 
  65  |     // 有菜单项（7 个 item，3 个 divider/label）
  66  |     const items = popover.locator('.ip-dropdown__item')
  67  |     await expect(items).toHaveCount(7)
  68  |   })
  69  | 
  70  |   test('2. hover 菜单项 → 高亮', async ({ page }) => {
  71  |     const trigger = ddTrigger(page)
  72  |     const popover = ddPopover(page)
  73  | 
  74  |     await trigger.click()
  75  |     await expect(popover).toBeVisible()
  76  | 
  77  |     // hover 第一个菜单项
  78  |     const firstItem = popover.locator('.ip-dropdown__item').first()
  79  |     await firstItem.hover()
  80  | 
  81  |     // 高亮 class 出现
  82  |     await expect(firstItem).toHaveClass(/ip-dropdown__item--focused/)
  83  |   })
  84  | 
  85  |   test('3. 点击菜单项 → 菜单关闭', async ({ page }) => {
  86  |     const trigger = ddTrigger(page)
  87  |     const popover = ddPopover(page)
  88  | 
  89  |     await trigger.click()
  90  |     await expect(popover).toBeVisible()
  91  | 
  92  |     // 点击第一个菜单项 "复制"
  93  |     await popover.locator('.ip-dropdown__item').first().click()
  94  | 
  95  |     // 菜单关闭
> 96  |     await expect(popover).not.toBeVisible()
      |                               ^ Error: expect(locator).not.toBeVisible() failed
  97  |   })
  98  | 
  99  |   test('4. 分隔线验证', async ({ page }) => {
  100 |     const trigger = ddTrigger(page)
  101 |     const popover = ddPopover(page)
  102 | 
  103 |     await trigger.click()
  104 |     await expect(popover).toBeVisible()
  105 | 
  106 |     // 分隔线存在且有 role=separator
  107 |     const dividers = popover.locator('.ip-dropdown__divider')
  108 |     const count = await dividers.count()
  109 |     expect(count).toBeGreaterThanOrEqual(1)
  110 | 
  111 |     // 第一个分隔线
  112 |     const firstDivider = dividers.first()
  113 |     await expect(firstDivider).toHaveAttribute('role', 'separator')
  114 |   })
  115 | 
  116 |   test('5. 点击外部 → 关闭', async ({ page }) => {
  117 |     const trigger = ddTrigger(page)
  118 |     const popover = ddPopover(page)
  119 | 
  120 |     await trigger.click()
  121 |     await expect(popover).toBeVisible()
  122 | 
  123 |     // 点击页面左上角空白区域
  124 |     await page.mouse.click(10, 10)
  125 | 
  126 |     await expect(popover).not.toBeVisible()
  127 |   })
  128 | 
  129 |   test('6. 按 Escape → 关闭', async ({ page }) => {
  130 |     const trigger = ddTrigger(page)
  131 |     const popover = ddPopover(page)
  132 | 
  133 |     await trigger.click()
  134 |     await expect(popover).toBeVisible()
  135 | 
  136 |     await page.keyboard.press('Escape')
  137 | 
  138 |     await expect(popover).not.toBeVisible()
  139 |   })
  140 | })
  141 | 
```