import { mount } from 'svelte';
import NotifDetached from './NotifDetached.svelte';
import './styles.css';

const target = document.getElementById('app');
if (!target) {
  throw new Error('Rift notif-detached bootstrap: #app element missing');
}

const app = mount(NotifDetached, { target });

export default app;
