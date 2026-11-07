import { saveDive } from '../divelog.ts';
import type { DiveBase } from '../divelog.ts';
import { h } from '../dom.ts';
import { describeItems, itemLabel, shortName, tokenOf } from '../items.ts';
import type { Session } from '../keys.ts';
import { drawView, setupCanvas } from '../render.ts';
import { DiveSession, MOVE, rulesInfo } from '../sim.ts';
import type { DiveOutcome, DiveView, GameEvent } from '../sim.ts';
import type { GraveView } from '../world.ts';

export interface DiveFinish {
  moves: Uint8Array;
  /** What the local run says would happen if the log ends here. */
  local: DiveOutcome;
  history: GameEvent[];
  reason: 'returned' | 'died' | 'limit';
}

export interface DiveScreenOptions {
  session: Session;
  base: DiveBase;
  /** Graves of the world at dive start, to name whose grave a looted one was. */
  graves: GraveView[];
  /** Moves already played (a resumed dive). */
  resume: Uint8Array;
  onFinish: (f: DiveFinish) => void;
}

const KEYS: Record<string, number> = {
  ArrowUp: MOVE.up, w: MOVE.up, W: MOVE.up,
  ArrowDown: MOVE.down, s: MOVE.down, S: MOVE.down,
  ArrowLeft: MOVE.left, a: MOVE.left, A: MOVE.left,
  ArrowRight: MOVE.right, d: MOVE.right, D: MOVE.right,
  ' ': MOVE.wait, '.': MOVE.wait,
};

function describe(ev: GameEvent, graveById: Map<number, GraveView>): string | null {
  switch (ev.kind) {
    case 'attacked': return ev.killed ? `You slay the monster (${ev.damage}).` : `You hit the monster for ${ev.damage}.`;
    case 'hurt': return `A monster hits you for ${ev.damage}.`;
    case 'missed': return 'A monster swings and misses.';
    case 'picked': return `You pick up ${ev.amount} ${itemLabel(tokenOf(ev.item))}.`;
    case 'grave_looted': {
      const g = graveById.get(ev.grave_id);
      return g ? `You empty ${shortName(g.owner_id)}'s grave: ${describeItems(g.items)}.` : 'You empty a grave.';
    }
    case 'blade_broke': return 'Your blade breaks. It is gone for good.';
    case 'potion_drunk': return `You drink a potion (+${ev.healed} HP).`;
    case 'door_opened': return 'The blue key turns. The door opens.';
    case 'door_locked': return 'The door is locked. It needs a blue key.';
    case 'descended': return `You go down to depth ${ev.depth}.`;
    case 'died': return 'You fall. What you carried stays here.';
    default: return null;
  }
}

export function diveScreen(opts: DiveScreenOptions): HTMLElement {
  const { session, base } = opts;
  const { ticket, entry } = base;
  const info = rulesInfo();
  const graveById = new Map(opts.graves.map((g) => [Number(g.id), g]));
  const sim = new DiveSession(BigInt(ticket.seed), entry);
  const moves: number[] = [];
  const history: GameEvent[] = [];
  let finished = false;

  const canvas = h('canvas', { role: 'img', 'aria-label': 'Dungeon map' });
  const ctx = setupCanvas(canvas, 32);
  const hpFill = h('i');
  const hpText = h('span');
  const hpBar = h('div', { class: 'bar', role: 'meter', 'aria-label': 'Hit points' }, hpFill, hpText);
  const depthChip = h('span', { class: 'chip' });
  const movesChip = h('span', { class: 'chip' });
  const killsChip = h('span', { class: 'chip' });
  const inventory = h('div', { class: 'chips', 'aria-label': 'You carry' });
  const log = h('div', { class: 'log', 'aria-live': 'polite' });
  const overlay = h('div', { class: 'banner bad', hidden: true });
  const returnBtn = h('button', { class: 'primary', type: 'button' }, 'Return to the surface');
  const stage = h('div', { class: 'stage' }, canvas);

  const lines: string[] = [];
  let flashUntil = 0;

  function paint(v: DiveView): void {
    drawView(ctx, v, performance.now() < flashUntil);
    hpFill.style.width = `${Math.max(0, Math.min(100, (v.hp / v.max_hp) * 100))}%`;
    hpBar.classList.toggle('low', v.hp / v.max_hp < 0.35);
    hpBar.setAttribute('aria-valuenow', String(v.hp));
    hpBar.setAttribute('aria-valuemax', String(v.max_hp));
    hpText.textContent = `${v.hp} / ${v.max_hp} HP`;
    depthChip.textContent = `Depth ${v.depth}`;
    movesChip.textContent = `Move ${v.moves_applied} / ${info.maxMoves}`;
    killsChip.textContent = `Kills ${v.kills}`;
    const chips = v.inventory.map(([id, n]) => h('span', { class: 'chip' }, `${itemLabel(tokenOf(id))} x${n}`));
    if (v.blade_durability > 0) chips.push(h('span', { class: 'chip' }, `Blade: ${v.blade_durability} hits left`));
    inventory.replaceChildren(...chips);
    log.textContent = lines.slice(-3).join(' ');
  }

  function save(): void {
    saveDive(session.accountId, base, Uint8Array.from(moves));
  }

  function finish(reason: DiveFinish['reason']): void {
    if (finished) return;
    finished = true;
    document.removeEventListener('keydown', onKey);
    const local = sim.outcome();
    const result: DiveFinish = { moves: Uint8Array.from(moves), local, history, reason };
    sim.free();
    opts.onFinish(result);
  }

  function play(mv: number): void {
    if (finished) return;
    const v0 = sim.view();
    if (v0.over) return;
    const { applied, events } = sim.step(mv);
    if (!applied) return;
    moves.push(mv);
    for (const ev of events) {
      history.push(ev);
      const text = describe(ev, graveById);
      if (text) lines.push(text);
      if (ev.kind === 'hurt') flashUntil = performance.now() + 160;
    }
    const v = sim.view();
    paint(v);
    save();
    if (flashUntil > performance.now()) setTimeout(() => !finished && paint(sim.view()), 180);
    if (v.over) {
      returnBtn.hidden = true;
      overlay.hidden = false;
      overlay.textContent = v.died
        ? 'You died. Everything you carried stays at the spot where you fell, as your grave. Recording the dive...'
        : 'You are out of moves for this dive. Recording the dive...';
      setTimeout(() => finish(v.died ? 'died' : 'limit'), 1200);
    }
  }

  function onKey(ev: KeyboardEvent): void {
    if (ev.ctrlKey || ev.metaKey || ev.altKey) return;
    const mv = KEYS[ev.key];
    if (mv === undefined) return;
    ev.preventDefault();
    play(mv);
  }

  returnBtn.addEventListener('click', () => finish('returned'));
  document.addEventListener('keydown', onKey);

  // Replay a resumed dive silently, then show the current state.
  for (const mv of opts.resume) {
    const { applied, events } = sim.step(mv);
    if (!applied) break;
    moves.push(mv);
    history.push(...events);
  }
  if (opts.resume.length > 0) lines.push(`Dive resumed at move ${moves.length}.`);
  const v = sim.view();
  paint(v);
  if (v.over) setTimeout(() => finish(v.died ? 'died' : 'limit'), 0);

  const pad = h('div', { class: 'pad', 'aria-label': 'Touch controls' },
    h('span'), padButton('Up', '↑', MOVE.up), h('span'),
    padButton('Left', '←', MOVE.left), padButton('Wait', '·', MOVE.wait), padButton('Right', '→', MOVE.right),
    h('span'), padButton('Down', '↓', MOVE.down), h('span'),
  );
  function padButton(label: string, glyph: string, mv: number): HTMLButtonElement {
    return h('button', { type: 'button', 'aria-label': label, onclick: () => play(mv) }, glyph);
  }

  return h('div', {},
    h('div', { class: 'row between' }, h('h2', {}, `${shortName(session.accountId)}'s dive`), h('span', { class: 'muted small' }, `Seed of the day ${ticket.day}`)),
    h('div', { class: 'hud' }, hpBar, depthChip, movesChip, killsChip, returnBtn),
    stage,
    overlay,
    h('div', { class: 'hud' }, h('strong', {}, 'You carry:'), inventory),
    log,
    pad,
    h('p', { class: 'muted small' }, 'Arrow keys or WASD to move, space to wait. Walk into a monster to attack. Go down the green stairs for better loot and older graves.'),
  );
}
