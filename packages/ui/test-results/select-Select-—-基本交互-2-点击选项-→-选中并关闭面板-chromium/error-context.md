# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: select.spec.ts >> Select — 基本交互 >> 2. 点击选项 → 选中并关闭面板
- Location: e2e/select.spec.ts:63:3

# Error details

```
Error: expect(locator).toBeVisible() failed

Locator: getByTestId('select-basic-wrap').getByRole('listbox')
Expected: visible
Timeout: 5000ms
Error: element(s) not found

Call log:
  - Expect "toBeVisible" with timeout 5000ms
  - waiting for getByTestId('select-basic-wrap').getByRole('listbox')

```

```yaml
- heading "E2E Fixtures — Select" [level=1]
- combobox [expanded]: 选择语气
- combobox: 选择一个模型
- combobox [disabled]: 禁用状态
- combobox: 选择一个模型
- button "触发错误"
- heading "E2E Fixtures — Popconfirm" [level=1]
- button "删除"
- button "删除"
- heading "E2E Fixtures — DropdownMenu" [level=1]
- button "操作"
- listbox "选项":
  - option "简洁 简短直接的回复"
  - option "友好 带情感温度的回复"
  - option "正式 商务场合的回复"
```

# Test source

```ts
  1   | /**
  2   |  * Select E2E 测试
  3   |  *
  4   |  * 测试场景：
  5   |  *  1. 点击 trigger 验证下拉面板展开
  6   |  *  2. 点击选项验证选中并面板关闭
  7   |  *  3. 再次点击 trigger 验证面板关闭
  8   |  *  4. 点击外部验证面板关闭
  9   |  *  5. 按 Escape 验证面板关闭
  10  |  *  6. 选中后验证 clearable 按钮出现
  11  |  *  7. 点击 clear 验证清空
  12  |  *  8. disabled 状态验证不可展开
  13  |  *  9. error 状态验证红色边框
  14  |  *
  15  |  * 预览站：必须运行于 http://localhost:5173
  16  |  * 夹具：/test/fixtures
  17  |  */
  18  | 
  19  | import { test, expect } from '@playwright/test'
  20  | 
  21  | /* ──────────────────────────────────────────────
  22  |  * Helper
  23  |  * ────────────────────────────────────────────── */
  24  | async function openSelect(page: import('@playwright/test').Page): Promise<void> {
  25  |   await page.getByTestId('select-basic-wrap').getByRole('combobox').click()
  26  | }
  27  | 
  28  | function selectLocator(page: import('@playwright/test').Page) {
  29  |   return page.getByTestId('select-basic-wrap').getByRole('combobox')
  30  | }
  31  | 
  32  | function popoverLocator(page: import('@playwright/test').Page) {
  33  |   return page.getByTestId('select-basic-wrap').getByRole('listbox')
  34  | }
  35  | 
  36  | function optionLocator(page: import('@playwright/test').Page, label: string) {
  37  |   return page.getByTestId('select-basic-wrap').getByRole('option', { name: label })
  38  | }
  39  | 
  40  | /* ──────────────────────────────────────────────
  41  |  * Tests
  42  |  * ────────────────────────────────────────────── */
  43  | 
  44  | test.describe('Select — 基本交互', () => {
  45  |   test.beforeEach(async ({ page }) => {
  46  |     await page.goto('/test/fixtures')
  47  |     // 确认 select section 加载
  48  |     await expect(page.getByTestId('fixture-select')).toBeVisible()
  49  |   })
  50  | 
  51  |   test('1. 点击 trigger 展开下拉面板', async ({ page }) => {
  52  |     const trigger = selectLocator(page)
  53  |     await trigger.click()
  54  | 
  55  |     // 面板展开
  56  |     const popover = popoverLocator(page)
  57  |     await expect(popover).toBeVisible()
  58  | 
  59  |     // aria-expanded 状态
  60  |     await expect(trigger).toHaveAttribute('aria-expanded', 'true')
  61  |   })
  62  | 
  63  |   test('2. 点击选项 → 选中并关闭面板', async ({ page }) => {
  64  |     const trigger = selectLocator(page)
  65  |     const popover = popoverLocator(page)
  66  | 
  67  |     // 打开面板
  68  |     await trigger.click()
> 69  |     await expect(popover).toBeVisible()
      |                           ^ Error: expect(locator).toBeVisible() failed
  70  | 
  71  |     // 点击选项 "友好"
  72  |     await optionLocator(page, '友好').click()
  73  | 
  74  |     // 面板关闭
  75  |     await expect(popover).not.toBeVisible()
  76  |     await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  77  | 
  78  |     // trigger 显示选中值
  79  |     await expect(trigger).toContainText('友好')
  80  |   })
  81  | 
  82  |   test('3. 再次点击 trigger → 关闭面板', async ({ page }) => {
  83  |     const trigger = selectLocator(page)
  84  |     const popover = popoverLocator(page)
  85  | 
  86  |     // 打开
  87  |     await trigger.click()
  88  |     await expect(popover).toBeVisible()
  89  | 
  90  |     // 再点 trigger，关闭
  91  |     await trigger.click()
  92  |     await expect(popover).not.toBeVisible()
  93  |   })
  94  | 
  95  |   test('4. 点击外部 → 关闭面板', async ({ page }) => {
  96  |     const trigger = selectLocator(page)
  97  |     const popover = popoverLocator(page)
  98  | 
  99  |     await trigger.click()
  100 |     await expect(popover).toBeVisible()
  101 | 
  102 |     // 点击 body 外部区域
  103 |     await page.click('body', { position: { x: 10, y: 10 } })
  104 |     await expect(popover).not.toBeVisible()
  105 |   })
  106 | 
  107 |   test('5. 按 Escape → 关闭面板', async ({ page }) => {
  108 |     const trigger = selectLocator(page)
  109 |     const popover = popoverLocator(page)
  110 | 
  111 |     await trigger.click()
  112 |     await expect(popover).toBeVisible()
  113 | 
  114 |     // 按 Escape
  115 |     await page.keyboard.press('Escape')
  116 |     await expect(popover).not.toBeVisible()
  117 |   })
  118 | })
  119 | 
  120 | test.describe('Select — Clearable', () => {
  121 |   test.beforeEach(async ({ page }) => {
  122 |     await page.goto('/test/fixtures')
  123 |     await expect(page.getByTestId('fixture-select')).toBeVisible()
  124 |   })
  125 | 
  126 |   test('6. 选中后 clear 按钮可见', async ({ page }) => {
  127 |     // clearable select 预选了 opt-b，对应 "Claude 3.5"
  128 |     const clearableWrap = page.getByTestId('select-clearable-wrap')
  129 |     const trigger = clearableWrap.getByRole('combobox')
  130 | 
  131 |     // hover 触发 clear 按钮出现
  132 |     await trigger.hover()
  133 |     const clearBtn = clearableWrap.getByRole('button', { name: '清空' })
  134 |     await expect(clearBtn).toBeVisible()
  135 |   })
  136 | 
  137 |   test('7. 点击 clear → 清空选中值', async ({ page }) => {
  138 |     const clearableWrap = page.getByTestId('select-clearable-wrap')
  139 |     const trigger = clearableWrap.getByRole('combobox')
  140 | 
  141 |     // hover 显示 clear 按钮
  142 |     await trigger.hover()
  143 |     const clearBtn = clearableWrap.getByRole('button', { name: '清空' })
  144 |     await clearBtn.click()
  145 | 
  146 |     // trigger 回到 placeholder
  147 |     await expect(trigger).toContainText('选择一个模型')
  148 |   })
  149 | })
  150 | 
  151 | test.describe('Select — 状态', () => {
  152 |   test.beforeEach(async ({ page }) => {
  153 |     await page.goto('/test/fixtures')
  154 |     await expect(page.getByTestId('fixture-select')).toBeVisible()
  155 |   })
  156 | 
  157 |   test('8. disabled 状态不可展开', async ({ page }) => {
  158 |     const disabledWrap = page.getByTestId('select-disabled-wrap')
  159 |     const trigger = disabledWrap.getByRole('combobox')
  160 | 
  161 |     // disabled 属性存在
  162 |     await expect(trigger).toHaveAttribute('aria-disabled', 'true')
  163 | 
  164 |     // 点击不展开（listbox 不存在）
  165 |     await trigger.click()
  166 |     await expect(disabledWrap.getByRole('listbox')).not.toBeVisible()
  167 |   })
  168 | 
  169 |   test('9. error 状态红色边框', async ({ page }) => {
```