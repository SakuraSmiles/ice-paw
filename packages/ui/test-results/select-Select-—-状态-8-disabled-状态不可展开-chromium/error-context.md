# Instructions

- Following Playwright test failed.
- Explain why, be concise, respect Playwright best practices.
- Provide a snippet of code with the fix, if possible.

# Test info

- Name: select.spec.ts >> Select — 状态 >> 8. disabled 状态不可展开
- Location: e2e/select.spec.ts:162:3

# Error details

```
TimeoutError: locator.click: Timeout 10000ms exceeded.
Call log:
  - waiting for getByTestId('select-disabled-wrap').getByRole('combobox')
    - locator resolved to <div tabindex="-1" role="combobox" data-v-418b800c="" aria-disabled="true" aria-expanded="false" aria-haspopup="listbox" aria-controls="ip-select-v-2-listbox" class="ip-select__trigger ip-select__trigger--md">…</div>
  - attempting click action
    2 × waiting for element to be visible, enabled and stable
      - element is not enabled
    - retrying click action
    - waiting 20ms
    2 × waiting for element to be visible, enabled and stable
      - element is not enabled
    - retrying click action
      - waiting 100ms
    19 × waiting for element to be visible, enabled and stable
       - element is not enabled
     - retrying click action
       - waiting 500ms

```

# Page snapshot

```yaml
- generic [ref=e3]:
  - generic [ref=e4]:
    - heading "E2E Fixtures — Select" [level=1] [ref=e5]
    - combobox [ref=e8] [cursor=pointer]:
      - generic [ref=e9]: 选择语气
      - img [ref=e10]
    - combobox [ref=e14] [cursor=pointer]:
      - generic [ref=e15]: 选择一个模型
      - img [ref=e16]
    - combobox [disabled] [ref=e20]:
      - generic [ref=e21]: 禁用状态
      - img [ref=e22]
    - generic [ref=e24]:
      - combobox [ref=e26] [cursor=pointer]:
        - generic [ref=e27]: 选择一个模型
        - img [ref=e28]
      - button "触发错误" [ref=e30] [cursor=pointer]
  - generic [ref=e31]:
    - heading "E2E Fixtures — Popconfirm" [level=1] [ref=e32]
    - button "删除" [ref=e36] [cursor=pointer]:
      - generic [ref=e37]: 删除
    - button "删除" [ref=e41] [cursor=pointer]:
      - generic [ref=e42]:
        - img [ref=e43]
        - text: 删除
  - generic [ref=e46]:
    - heading "E2E Fixtures — DropdownMenu" [level=1] [ref=e47]
    - button "操作" [ref=e51] [cursor=pointer]:
      - generic [ref=e52]: 操作
```

# Test source

```ts
  70  |     const popover = basicPopover(page)
  71  | 
  72  |     // 打开面板
  73  |     await trigger.click()
  74  |     await expect(popover).toBeVisible()
  75  | 
  76  |     // 点击选项 "友好"
  77  |     await basicOption(page, '友好').click()
  78  | 
  79  |     // 面板关闭
  80  |     await expect(popover).not.toBeVisible()
  81  |     await expect(trigger).toHaveAttribute('aria-expanded', 'false')
  82  | 
  83  |     // trigger 显示选中值
  84  |     await expect(trigger).toContainText('友好')
  85  |   })
  86  | 
  87  |   test('3. 再次点击 trigger → 关闭面板', async ({ page }) => {
  88  |     const trigger = basicTrigger(page)
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
> 170 |     await trigger.click()
      |                   ^ TimeoutError: locator.click: Timeout 10000ms exceeded.
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
  189 |     await expect(trigger).toHaveCSS('border-color', 'rgb(239, 68, 68)')
  190 |   })
  191 | })
  192 | 
```