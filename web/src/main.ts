import './style.css';
import { explorerTxUrl } from './chain.ts';
import { clearDive, saveDive } from './divelog.ts';
import type { DiveBase, StartInfo } from './divelog.ts';
import { h, mount } from './dom.ts';
import { forgetLast, lastName } from './keys.ts';
import type { Session } from './keys.ts';
import { diveScreen, replayFinish } from './screens/dive.ts';
import type { DiveFinish } from './screens/dive.ts';
import { loginScreen } from './screens/login.ts';
import { resultScreen } from './screens/result.ts';
import { surfaceScreen } from './screens/surface.ts';
import type { Flash } from './screens/surface.ts';
import { loadRules, rulesInfo } from './sim.ts';
import type { DiveEntry } from './sim.ts';
import { storedSecret } from './keys.ts';
import { config } from './config.ts';
import { allGraves, diveEntry, settleDive, startDive } from './world.ts';
import type { DiveTicket, GraveView } from './world.ts';
import type { SavedDive } from './divelog.ts';
import type { CallResult } from './tx.ts';

const root = document.getElementById('app')!;

function footer(): HTMLElement {
  const i = rulesInfo();
  return h('footer', { class: 'muted small' },
    `NEAR testnet only. No real money. World: ${config.worldContractId}. Rules ${i.version}, hash ${i.rulesHash.slice(0, 12)}.`);
}

function busy(text: string): void {
  mount(root, h('div', { class: 'card', role: 'status' }, text));
}

function showLogin(): void {
  mount(root, loginScreen(showSurface, lastName()), footer());
}

function showSurface(session: Session, flash?: Flash): void {
  mount(
    root,
    surfaceScreen(
      session,
      {
        onDive: (loadout) => void beginDive(session, loadout),
        onContinue: (ticket, saved) => void resumeDive(session, ticket, saved),
        onEnd: (ticket, saved) => void endDive(session, ticket, saved),
        onSwitchPlayer: () => { forgetLast(); showLogin(); },
      },
      flash,
    ),
    footer(),
  );
}

async function entryFor(session: Session): Promise<{ entry: DiveEntry; graves: GraveView[] }> {
  const [entry, graves] = await Promise.all([diveEntry(session.accountId), allGraves()]);
  // `dive_entry` is exactly what settle_dive replays: the locked loadout and the graves, in stored order.
  return { entry, graves };
}

async function beginDive(session: Session, loadout: Array<[string, number]>): Promise<void> {
  busy('Writing your loadout to the chain...');
  let start: { ticket: DiveTicket; call: CallResult };
  try {
    start = await startDive(session, loadout);
  } catch (e) {
    const msg = (e as Error).message;
    showSurface(session, {
      kind: 'bad',
      text: /does not exist/.test(msg)
        ? 'Your account is not on the chain (the testnet may have been reset). Use "Switch player" and enter the same name to create it again.'
        : `Could not start the dive: ${msg}`,
    });
    return;
  }
  try {
    const { entry, graves } = await entryFor(session);
    const base: DiveBase = { ticket: start.ticket, entry, start: { txHash: start.call.txHash, via: start.call.via } };
    saveDive(session.accountId, base, new Uint8Array());
    runDive(session, base, graves, new Uint8Array());
  } catch (e) {
    showSurface(session, { kind: 'bad', text: `The dive started, but its starting state could not be read: ${(e as Error).message}` });
  }
}

async function resumeDive(session: Session, ticket: DiveTicket, saved: { saved: SavedDive; moves: Uint8Array } | null): Promise<void> {
  busy('Opening your dive...');
  try {
    const graves = await allGraves();
    if (saved) {
      runDive(session, saved.saved, graves, saved.moves);
    } else {
      const { entry } = await entryFor(session);
      runDive(session, { ticket, entry, start: null }, graves, new Uint8Array());
    }
  } catch (e) {
    showSurface(session, { kind: 'bad', text: `Could not open the dive: ${(e as Error).message}` });
  }
}

/** Records an open dive without playing on: the saved moves, or an empty log if this browser has none. */
async function endDive(session: Session, ticket: DiveTicket, saved: { saved: SavedDive; moves: Uint8Array } | null): Promise<void> {
  busy('Reading your dive...');
  let graves: GraveView[];
  let base: DiveBase;
  let finish: DiveFinish;
  try {
    graves = await allGraves();
    base = saved ? saved.saved : { ticket, entry: (await entryFor(session)).entry, start: null };
    finish = replayFinish(base, saved ? saved.moves : new Uint8Array());
  } catch (e) {
    showSurface(session, { kind: 'bad', text: `Could not read the dive: ${(e as Error).message}` });
    return;
  }
  await record(session, ticket, graves, finish, base.start);
}

function runDive(session: Session, base: DiveBase, graves: GraveView[], resume: Uint8Array): void {
  mount(root, diveScreen({
    session, base, graves, resume,
    onFinish: (finish) => void record(session, base.ticket, graves, finish, base.start),
  }), footer());
}

async function record(session: Session, ticket: DiveTicket, graves: GraveView[], finish: DiveFinish, start: StartInfo | null): Promise<void> {
  busy('Recording your dive on the chain. The contract replays your moves with the same rules...');
  try {
    const { outcome, call } = await settleDive(session, ticket.dive_id, finish.moves);
    clearDive(session.accountId);
    mount(root, resultScreen({ start, settle: call, chain: outcome, finish, graves }, () => showSurface(session, {
      kind: 'good',
      text: 'Dive recorded.',
      links: call.txHash ? [{ label: 'View transaction', url: explorerTxUrl(call.txHash) }] : [],
    })), footer());
  } catch (e) {
    mount(root,
      h('div', { class: 'banner bad', role: 'alert' }, `Could not record the dive: ${(e as Error).message}`),
      h('p', {}, 'Your moves are saved in this browser. Nothing is lost: try again.'),
      h('div', { class: 'row' },
        h('button', { class: 'primary', type: 'button', onclick: () => void record(session, ticket, graves, finish, start) }, 'Try again'),
        h('button', { type: 'button', onclick: () => showSurface(session) }, 'Back to the surface')),
      footer());
  }
}

async function boot(): Promise<void> {
  busy('Loading the rules...');
  try {
    await loadRules();
  } catch (e) {
    mount(root, h('div', { class: 'banner bad', role: 'alert' }, `The game rules could not be loaded: ${(e as Error).message}`));
    return;
  }
  // Resume the last player of this browser without asking for the name again.
  const name = lastName();
  const secret = name ? storedSecret(name) : null;
  if (name && secret) showSurface({ name, accountId: `${name}.${config.parentAccountId}`, secretKey: secret });
  else showLogin();
}

void boot();
