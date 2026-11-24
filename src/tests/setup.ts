import { config } from '@vue/test-utils';

// Page tests exercise the public CustomSelect contract. Its own tests mount real Element Plus controls.
config.global.stubs.ElSelect = {
  props: ['modelValue', 'ariaLabel'],
  template: '<div :aria-label="ariaLabel"><slot name="header"/><slot name="tag"/><slot/></div>',
};
config.global.stubs.ElOption = { template: '<span><slot/></span>' };
