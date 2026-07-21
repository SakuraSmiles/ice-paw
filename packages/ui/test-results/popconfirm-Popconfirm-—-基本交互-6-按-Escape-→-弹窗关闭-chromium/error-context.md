# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: popconfirm.spec.ts >> Popconfirm — 基本交互 >> 6. 按 Escape → 弹窗关闭
- Location: e2e/popconfirm.spec.ts:143:3

# Error details

```
Error: expect(locator).not.toBeVisible() failed

Locator:  locator('.ip-popconfirm__popover').first()
Expected: not visible
Received: visible
Timeout:  5000ms

Call log:
  - Expect "not toBeVisible" with timeout 5000ms
  - waiting for locator('.ip-popconfirm__popover').first()
    14 × locator resolved to <div data-v-5586a090="" role="alertdialog" aria-modal="false" id="ip-popconfirm-v-4" class="ip-popconfirm__popover" aria-labelledby="ip-popconfirm-v-4-title" aria-describedby="ip-popconfirm-v-4-desc">…</div>
       - unexpected value "visible"

```

```yaml
- alertdialog "确定要删除吗？":
  - text: 确定要删除吗？ 此操作不可撤销。
  - button "取消"
  - button "确认"
```

# Test source

```ts
  52  | test.describe('Popconfirm — 基本交互', () => {
  53  |   test.beforeEach(async ({ page }) => {
  54  |     await page.goto('/test/fixtures')
  55  |     await expect(page.getByTestId('fixture-popconfirm')).toBeVisible()
  56  |   })
  57  | 
  58  |   test('1. 点击触发按钮 → 弹窗出现', async ({ page }) => {
  59  |     const trigger = basicTrigger(page)
  60  |     const popover = basicPopover(page)
  61  | 
  62  |     // 初始弹窗不可见
  63  |     await expect(popover).not.toBeVisible()
  64  | 
  65  |     // 点击触发按钮
  66  |     await trigger.click()
  67  | 
  68  |     // 弹窗出现
  69  |     await expect(popover).toBeVisible()
  70  | 
  71  |     // role=alertdialog
  72  |     await expect(popover).toHaveAttribute('role', 'alertdialog')
  73  | 
  74  |     // trigger aria-expanded
  75  |     await expect(trigger).toHaveAttribute('aria-expanded', 'true')
  76  |   })
  77  | 
  78  |   test('2. 弹窗内容验证（标题、描述、确认/取消按钮）', async ({ page }) => {
  79  |     const trigger = basicTrigger(page)
  80  |     const popover = basicPopover(page)
  81  | 
  82  |     await trigger.click()
  83  |     await expect(popover).toBeVisible()
  84  | 
  85  |     // 标题
  86  |     await expect(popover.locator('.ip-popconfirm__title')).toContainText('确定要删除吗？')
  87  | 
  88  |     // 描述
  89  |     await expect(popover.locator('.ip-popconfirm__description')).toContainText('此操作不可撤销。')
  90  | 
  91  |     // 确认按钮（variant=primary）
  92  |     const confirmBtn = popover.locator('.ip-button--primary')
  93  |     await expect(confirmBtn).toBeVisible()
  94  |     await expect(confirmBtn).toContainText('确认')
  95  | 
  96  |     // 取消按钮（variant=secondary）
  97  |     const cancelBtn = popover.locator('.ip-button--secondary')
  98  |     await expect(cancelBtn).toBeVisible()
  99  |     await expect(cancelBtn).toContainText('取消')
  100 |   })
  101 | 
  102 |   test('3. 点击确认 → 事件触发 + 弹窗关闭', async ({ page }) => {
  103 |     const trigger = basicTrigger(page)
  104 |     const popover = basicPopover(page)
  105 | 
  106 |     await trigger.click()
  107 |     await expect(popover).toBeVisible()
  108 | 
  109 |     // 点击确认按钮
  110 |     await popover.locator('.ip-button--primary').click()
  111 | 
  112 |     // 弹窗关闭
  113 |     await expect(popover).not.toBeVisible()
  114 |   })
  115 | 
  116 |   test('4. 点击取消 → 弹窗关闭', async ({ page }) => {
  117 |     const trigger = basicTrigger(page)
  118 |     const popover = basicPopover(page)
  119 | 
  120 |     await trigger.click()
  121 |     await expect(popover).toBeVisible()
  122 | 
  123 |     // 点击取消按钮
  124 |     await popover.locator('.ip-button--secondary').click()
  125 | 
  126 |     // 弹窗关闭
  127 |     await expect(popover).not.toBeVisible()
  128 |   })
  129 | 
  130 |   test('5. 点击外部 → 弹窗关闭', async ({ page }) => {
  131 |     const trigger = basicTrigger(page)
  132 |     const popover = basicPopover(page)
  133 | 
  134 |     await trigger.click()
  135 |     await expect(popover).toBeVisible()
  136 | 
  137 |     // 点击页面左上角空白区域
  138 |     await page.mouse.click(10, 10)
  139 | 
  140 |     await expect(popover).not.toBeVisible()
  141 |   })
  142 | 
  143 |   test('6. 按 Escape → 弹窗关闭', async ({ page }) => {
  144 |     const trigger = basicTrigger(page)
  145 |     const popover = basicPopover(page)
  146 | 
  147 |     await trigger.click()
  148 |     await expect(popover).toBeVisible()
  149 | 
  150 |     await page.keyboard.press('Escape')
  151 | 
> 152 |     await expect(popover).not.toBeVisible()
      |                               ^ Error: expect(locator).not.toBeVisible() failed
  153 |   })
  154 | })
  155 | 
  156 | test.describe('Popconfirm — 样式', () => {
  157 |   test.beforeEach(async ({ page }) => {
  158 |     await page.goto('/test/fixtures')
  159 |     await expect(page.getByTestId('fixture-popconfirm')).toBeVisible()
  160 |   })
  161 | 
  162 |   test('7. danger 样式确认按钮为危险色', async ({ page }) => {
  163 |     const trigger = dangerTrigger(page)
  164 |     const popover = dangerPopover(page)
  165 | 
  166 |     // 打开 danger 弹窗
  167 |     await trigger.click()
  168 |     await expect(popover).toBeVisible()
  169 | 
  170 |     // 确认按钮有 danger 变体（.ip-button--danger）
  171 |     const confirmBtn = popover.locator('.ip-button')
  172 |     await expect(confirmBtn).toHaveClass(/ip-button--danger/)
  173 |   })
  174 | })
  175 | 
```