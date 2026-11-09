import { accountExists } from '../chain.ts';
import { config, nameProblem } from '../config.ts';
import { h } from '../dom.ts';
import { KeyPair } from 'near-api-js';
import { newKeyPair, rememberKey, saveSession, storedSecret } from '../keys.ts';
import type { Session } from '../keys.ts';
import { RelayerRefused, RelayerUnavailable, signup } from '../relayer.ts';

/**
 * Signs in by name. A name this browser already holds a key for resumes at once (works with the
 * studio backend off); a new name needs the relayer to create <name>.<parent> with the browser's new key.
 */
async function signIn(name: string): Promise<Session> {
  const accountId = `${name}.${config.parentAccountId}`;
  const known = storedSecret(name);
  // A key kept in this browser resumes at once, as long as its account still exists on the chain
  // (an unreachable RPC counts as "exists"; a reset testnet does not, and the account is created again).
  if (known !== null && (await accountExists(accountId).catch(() => true))) return { name, accountId, secretKey: known };

  const key = known !== null ? KeyPair.fromString(known as `ed25519:${string}`) : newKeyPair();
  try {
    await signup(name, key.getPublicKey().toString());
  } catch (e) {
    if (e instanceof RelayerRefused && e.code === 'name_taken') {
      throw new Error('That name is taken. If it is yours, open the game in the browser where you created it.');
    }
    if (e instanceof RelayerRefused) throw new Error(e.message);
    if (e instanceof RelayerUnavailable) {
      // The relayer may have created the account before failing: keep the key, so trying again
      // with the same name signs in with it instead of losing the name.
      rememberKey(name, key.toString());
      // Be precise about why: the name may still be free; nobody can create it right now.
      const taken = await accountExists(accountId).catch(() => false);
      throw new Error(
        taken
          ? 'That name already exists. If you just tried to create it in this browser, press Play again to sign in.'
          : 'The studio server is offline, so new players cannot be created right now. Players who already have a key in this browser can still play.',
      );
    }
    throw e;
  }
  const session: Session = { name, accountId, secretKey: key.toString() };
  saveSession(session);
  return session;
}

export function loginScreen(onDone: (s: Session) => void, initialName: string): HTMLElement {
  const input = h('input', {
    type: 'text', id: 'name', name: 'name', autocomplete: 'username', autocapitalize: 'none', spellcheck: false,
    placeholder: 'your name, e.g. tom', value: initialName, 'aria-describedby': 'name-help',
  });
  const error = h('div', { class: 'banner bad', role: 'alert', hidden: true });
  const button = h('button', { class: 'primary', type: 'submit' }, 'Play');

  const form = h('form', { class: 'card login', novalidate: true },
    h('label', { for: 'name' }, h('strong', {}, 'Choose a name')),
    input,
    h('p', { id: 'name-help', class: 'muted small' }, 'No email, no wallet, no password. Your key stays in this browser.'),
    error,
    button,
  );
  form.addEventListener('submit', async (ev) => {
    ev.preventDefault();
    const name = input.value.trim().toLowerCase();
    const problem = nameProblem(name);
    error.hidden = problem === null;
    if (problem !== null) { error.textContent = problem; return; }
    button.disabled = true;
    button.textContent = 'Creating your character...';
    try {
      onDone(await signIn(name));
    } catch (e) {
      error.hidden = false;
      error.textContent = (e as Error).message;
      button.disabled = false;
      button.textContent = 'Play';
    }
  });

  return h('div', {},
    h('div', { class: 'hero' },
      h('h1', {}, 'Last Server'),
      h('p', {}, 'Dive into a dungeon. What you carry down stays where you fall, for the next player to take. The world lives on the NEAR testnet, so it keeps running after the studio switches its server off.'),
    ),
    form,
  );
}
