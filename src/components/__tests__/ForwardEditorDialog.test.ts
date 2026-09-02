import { mount } from '@vue/test-utils'
import ElementPlus from 'element-plus'
import ForwardEditorDialog from '../ForwardEditorDialog.vue'
import type { Forward } from '../../types'

function blank(): Forward {
  return {
    id: '', server_id: 's1', name: '', kind: 'local',
    bind_addr: '127.0.0.1', bind_port: 0, target_host: null, target_port: null,
    auto_start: false,
  }
}

// el-dialog 默认把内容 teleport 到 body,stub teleport 后渲染进 wrapper 树才能断言/查找
const globalOpts = { plugins: [ElementPlus], stubs: { teleport: true } }

describe('ForwardEditorDialog', () => {
  it('dynamic 类型隐藏目标字段', async () => {
    const wrapper = mount(ForwardEditorDialog, {
      props: { modelValue: true, forward: blank() },
      global: globalOpts,
    })
    // el-dialog 在 onMounted 才置 rendered=true,重渲染要等一个 tick
    await wrapper.vm.$nextTick()
    expect(wrapper.text()).toContain('目标地址')
    // 点击第三个 radio(SOCKS -D)走真实 change 链路,不探入组件 vm 内部
    await wrapper.findAll('input[type="radio"]')[2].setValue(true)
    expect(wrapper.text()).not.toContain('目标地址')
  })

  it('提交时校验:local/remote 必须填目标', async () => {
    const wrapper = mount(ForwardEditorDialog, {
      props: { modelValue: true, forward: blank() },
      global: globalOpts,
    })
    await wrapper.vm.$nextTick()
    await wrapper.find('form').trigger('submit.prevent')
    // 目标为空 → 不触发 submit
    expect(wrapper.emitted('submit')).toBeFalsy()
  })
})
