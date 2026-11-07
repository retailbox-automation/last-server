import './style.css';
import { h, mount } from './dom.ts';
import { lastName } from './keys.ts';
import type { Session } from './keys.ts';
import { loginScreen } from './screens/login.ts';

const root = document.getElementById('app')!;

function showLogin(): void {
  mount(root, loginScreen(showSurface, lastName()), footer());
}

function showSurface(session: Session): void {
  mount(root, h('div', { class: 'card' }, `Signed in as ${session.accountId}`), footer());
}

function footer(): HTMLElement {
  return h('footer', { class: 'muted small' }, 'NEAR testnet only. No real money.');
}

showLogin();
