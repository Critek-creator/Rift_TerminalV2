import { mount } from 'svelte';
import CockpitDetached from './CockpitDetached.svelte';
import './styles.css';

const target = document.getElementById('app');
if (!target) {
  throw new Error('Rift cockpit-detached bootstrap: #app element missing');
}

const app = mount(CockpitDetached, { target });

export default app;
