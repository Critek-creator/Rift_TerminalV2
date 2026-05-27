import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

export default {
  preprocess: vitePreprocess(),
  compilerOptions: {
    runes: true,
    warningFilter: (warning) => {
      // These elements are intentionally interactive (terminal panes, timeline
      // scrub, waterfall hover, file viewer) with proper ARIA roles + keyboard
      // handlers. Inline svelte-ignore comments exist but don't suppress
      // reliably across vite-plugin-svelte + svelte-check.
      if (warning.code === 'a11y_no_noninteractive_element_interactions') return false;
      if (warning.code === 'a11y_no_noninteractive_tabindex') return false;
      return true;
    },
  },
};
