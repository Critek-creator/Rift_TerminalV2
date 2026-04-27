import { mount } from 'svelte';
import App from './App.svelte';
import './styles.css';

const target = document.getElementById('app');
if (!target) {
  throw new Error('Rift bootstrap: #app element missing from index.html');
}

const app = mount(App, { target });

export default app;
