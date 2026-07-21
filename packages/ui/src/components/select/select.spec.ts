import { describe, it } from 'vitest'
import { mount } from '@vue/test-utils'
import Select from '@/components/select/Select.vue'

describe('Select bug reproduction (full DOM events)', () => {
  it('first click should open dropdown', async () => {
    const wrapper = mount(Select, {
      props: {
        modelValue: null,
        options: [
          { value: 'a', label: 'A' },
          { value: 'b', label: 'B' },
        ],
      },
      attachTo: document.body,
    })

    const trigger = wrapper.find('.ip-select__trigger').element as HTMLElement
    
    // Simulate full click sequence: mousedown -> mouseup -> click
    trigger.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    trigger.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }))
    trigger.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    
    await new Promise(r => setTimeout(r, 50))
    
    console.log('After full sequence aria-expanded:', wrapper.find('[role=combobox]').attributes('aria-expanded'))
    const popover = document.querySelector('.ip-select__popover')
    console.log('Popover in DOM:', !!popover)

    wrapper.unmount()
  })

  it('clicking second Select while first is open should close first, open second', async () => {
    const w1 = mount(Select, {
      props: { modelValue: null, options: [{ value: 'a', label: 'A' }] },
      attachTo: document.body,
    })
    const w2 = mount(Select, {
      props: { modelValue: null, options: [{ value: 'x', label: 'X' }] },
      attachTo: document.body,
    })

    // Open w1
    const t1 = w1.find('.ip-select__trigger').element as HTMLElement
    t1.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    t1.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }))
    t1.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await new Promise(r => setTimeout(r, 50))
    console.log('After open w1: w1 aria-expanded =', w1.find('[role=combobox]').attributes('aria-expanded'))

    // Click w2 trigger (mousedown should close w1)
    const t2 = w2.find('.ip-select__trigger').element as HTMLElement
    t2.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }))
    t2.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }))
    t2.dispatchEvent(new MouseEvent('click', { bubbles: true }))
    await new Promise(r => setTimeout(r, 50))
    console.log('After click w2: w1 aria-expanded =', w1.find('[role=combobox]').attributes('aria-expanded'))
    console.log('After click w2: w2 aria-expanded =', w2.find('[role=combobox]').attributes('aria-expanded'))

    w1.unmount()
    w2.unmount()
  })

  it('clicking trigger area (simulated - event target is the trigger div)', async () => {
    const wrapper = mount(Select, {
      props: { modelValue: null, options: [{ value: 'a', label: 'A' }] },
      attachTo: document.body,
    })

    // Just trigger the @click on the trigger div directly
    await wrapper.find('.ip-select__trigger').trigger('click')
    await new Promise(r => setTimeout(r, 50))
    console.log('After @click trigger: aria-expanded =', wrapper.find('[role=combobox]').attributes('aria-expanded'))

    wrapper.unmount()
  })
})
