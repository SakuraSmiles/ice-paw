# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: select.spec.ts >> Select — 状态 >> 9. error 状态红色边框
- Location: e2e/select.spec.ts:174:3

# Error details

```
Error: expect(locator).toHaveCSS(expected) failed

Locator:  getByTestId('select-error-wrap').getByRole('combobox')
Expected: "rgb(239, 68, 68)"
Received: "rgb(220, 38, 38)"
Timeout:  5000ms

Call log:
  - Expect "toHaveCSS" with timeout 5000ms
  - waiting for getByTestId('select-error-wrap').getByRole('combobox')
    - locator resolved to <div tabindex="0" role="combobox" data-v-418b800c="" aria-expanded="false" aria-haspopup="listbox" aria-controls="ip-select-v-3-listbox" class="ip-select__trigger ip-select__trigger--md">…</div>
    - unexpected value "rgb(217, 217, 217)"
    - locator resolved to <div tabindex="0" role="combobox" data-v-418b800c="" aria-expanded="false" aria-haspopup="listbox" aria-controls="ip-select-v-3-listbox" class="ip-select__trigger ip-select__trigger--md">…</div>
    - unexpected value "rgb(218, 138, 138)"
    - locator resolved to <div tabindex="0" role="combobox" data-v-418b800c="" aria-expanded="false" aria-haspopup="listbox" aria-controls="ip-select-v-3-listbox" class="ip-select__trigger ip-select__trigger--md">…</div>
    - unexpected value "rgb(220, 50, 50)"
    11 × locator resolved to <div tabindex="0" role="combobox" data-v-418b800c="" aria-expanded="false" aria-haspopup="listbox" aria-controls="ip-select-v-3-listbox" class="ip-select__trigger ip-select__trigger--md">…</div>
       - unexpected value "rgb(220, 38, 38)"

```

```yaml
- combobox: 选择一个模型
```

# Test source

```ts
  89  |     const popover = basicPopover(page)
  90  | 
  91  |     // 打开
  92  |     await trigger.click()
  93  |     await expect(popover).toBeVisible()
  94  | 
  95  |     // 再点 trigger，关闭
  96  |     await trigger.click()
  97  |     await expect(popover).not.toBeVisible()
  98  |   })
  99  | 
  100 |   test('4. 点击外部 → 关闭面板', async ({ page }) => {
  101 |     const trigger = basicTrigger(page)
  102 |     const popover = basicPopover(page)
  103 | 
  104 |     await trigger.click()
  105 |     await expect(popover).toBeVisible()
  106 | 
  107 |     // 点击页面左上角空白区域（fixture-select section 之外）
  108 |     await page.mouse.click(10, 10)
  109 |     await expect(popover).not.toBeVisible()
  110 |   })
  111 | 
  112 |   test('5. 按 Escape → 关闭面板', async ({ page }) => {
  113 |     const trigger = basicTrigger(page)
  114 |     const popover = basicPopover(page)
  115 | 
  116 |     await trigger.click()
  117 |     await expect(popover).toBeVisible()
  118 | 
  119 |     // 按 Escape
  120 |     await page.keyboard.press('Escape')
  121 |     await expect(popover).not.toBeVisible()
  122 |   })
  123 | })
  124 | 
  125 | test.describe('Select — Clearable', () => {
  126 |   test.beforeEach(async ({ page }) => {
  127 |     await page.goto('/test/fixtures')
  128 |     await expect(page.getByTestId('fixture-select')).toBeVisible()
  129 |   })
  130 | 
  131 |   test('6. 选中后 clear 按钮可见', async ({ page }) => {
  132 |     // clearable select 预选了 opt-b，对应 "Claude 3.5"
  133 |     const clearableWrap = page.getByTestId('select-clearable-wrap')
  134 |     const trigger = clearableWrap.getByRole('combobox')
  135 | 
  136 |     // hover 触发 clear 按钮出现
  137 |     await trigger.hover()
  138 |     const clearBtn = clearableWrap.getByRole('button', { name: '清空' })
  139 |     await expect(clearBtn).toBeVisible()
  140 |   })
  141 | 
  142 |   test('7. 点击 clear → 清空选中值', async ({ page }) => {
  143 |     const clearableWrap = page.getByTestId('select-clearable-wrap')
  144 |     const trigger = clearableWrap.getByRole('combobox')
  145 | 
  146 |     // hover 显示 clear 按钮
  147 |     await trigger.hover()
  148 |     const clearBtn = clearableWrap.getByRole('button', { name: '清空' })
  149 |     await clearBtn.click()
  150 | 
  151 |     // trigger 回到 placeholder
  152 |     await expect(trigger).toContainText('选择一个模型')
  153 |   })
  154 | })
  155 | 
  156 | test.describe('Select — 状态', () => {
  157 |   test.beforeEach(async ({ page }) => {
  158 |     await page.goto('/test/fixtures')
  159 |     await expect(page.getByTestId('fixture-select')).toBeVisible()
  160 |   })
  161 | 
  162 |   test('8. disabled 状态不可展开', async ({ page }) => {
  163 |     const disabledWrap = page.getByTestId('select-disabled-wrap')
  164 |     const trigger = disabledWrap.getByRole('combobox')
  165 | 
  166 |     // disabled 属性存在
  167 |     await expect(trigger).toHaveAttribute('aria-disabled', 'true')
  168 | 
  169 |     // 点击不展开（listbox 不存在）
  170 |     await trigger.click()
  171 |     await expect(disabledWrap.getByRole('listbox')).not.toBeVisible()
  172 |   })
  173 | 
  174 |   test('9. error 状态红色边框', async ({ page }) => {
  175 |     const errorWrap = page.getByTestId('select-error-wrap')
  176 |     const root = errorWrap.locator('.ip-select')
  177 | 
  178 |     // 初始无 error class
  179 |     await expect(root).not.toHaveClass(/ip-select--error/)
  180 | 
  181 |     // 触发错误
  182 |     await errorWrap.getByTestId('select-error-trigger').click()
  183 | 
  184 |     // error class 出现
  185 |     await expect(root).toHaveClass(/ip-select--error/)
  186 | 
  187 |     // trigger 有错误边框色
  188 |     const trigger = errorWrap.getByRole('combobox')
> 189 |     await expect(trigger).toHaveCSS('border-color', 'rgb(239, 68, 68)')
      |                           ^ Error: expect(locator).toHaveCSS(expected) failed
  190 |   })
  191 | })
  192 | 
```