import { loadDive } from '../divelog.ts';
import type { SavedDive } from '../divelog.ts';
import { h } from '../dom.ts';
import { describeTokens, itemLabel, shortName } from '../items.ts';
import type { Session } from '../keys.ts';
import { rulesInfo } from '../sim.ts';
import { activeDive, allGraves, coreVersion, currentDay, player } from '../world.ts';
import type { DiveTicket, GraveView, PlayerView } from '../world.ts';

export interface Flash {
  kind: 'good' | 'bad' | 'warn';
  text: string;
  links?: Array<{ label: string; url: string }>;
}

export interface SurfaceActions {
  onDive: (loadout: Array<[string, number]>) => void;
  onContinue: (ticket: DiveTicket, saved: { saved: SavedDive; moves: Uint8Array } | null) => void;
  /** Settle the open dive right away with the moves this browser has (none: an empty log). */
  onEnd: (ticket: DiveTicket, saved: { saved: SavedDive; moves: Uint8Array } | null) => void;
  onSwitchPlayer: () => void;
}

/** token id -> amount the player can put into a loadout (the contract's `stash` is already net of locked items). */
function available(p: PlayerView): Map<string, number> {
  return new Map(p.stash.map(([t, n]) => [t, Number(n)] as [string, number]).filter(([, n]) => n > 0));
}

function gravesCard(graves: GraveView[]): HTMLElement {
  const byDepth = new Map<number, GraveView[]>();
  for (const g of graves) byDepth.set(g.depth, [...(byDepth.get(g.depth) ?? []), g]);
  const deepest = Math.max(3, ...byDepth.keys());
  const rows: HTMLElement[] = [];
  for (let d = 1; d <= deepest; d++) {
    const here = byDepth.get(d) ?? [];
    rows.push(
      h('div', { class: here.length ? 'depth' : 'depth empty' },
        h('strong', {}, `Depth ${d}`),
        here.length
          ? h('div', {}, ...here.map((g) =>
              h('div', { class: 'grave' },
                h('span', { class: 'skull', 'aria-hidden': 'true' }, '☠'),
                h('strong', {}, shortName(g.owner_id)),
                h('span', {}, describeTokens(g.items)),
              )))
          : h('span', { class: 'muted' }, 'No graves here yet'),
      ),
    );
  }
  return h('section', { class: 'card' },
    h('h2', {}, 'Graves in the world'),
    h('p', { class: 'muted small' }, 'Read straight from the chain. Reach a grave first and what its owner carried is yours.'),
    h('div', { class: 'depths' }, ...rows),
  );
}

export function surfaceScreen(session: Session, actions: SurfaceActions, flash?: Flash): HTMLElement {
  const root = h('div', {}, h('p', { class: 'muted' }, 'Reading the world from the NEAR testnet...'));

  const header = h('div', { class: 'row between' },
    h('div', {}, h('h1', {}, 'Last Server'), h('span', { class: 'muted small mono' }, session.accountId)),
    h('button', { type: 'button', onclick: actions.onSwitchPlayer }, 'Switch player'),
  );

  async function load(): Promise<void> {
    let day: number, graves: GraveView[], me: PlayerView, active: DiveTicket | null;
    try {
      [day, graves, me, active] = await Promise.all([currentDay(), allGraves(), player(session.accountId), activeDive(session.accountId)]);
    } catch (e) {
      root.replaceChildren(
        header,
        h('div', { class: 'banner bad', role: 'alert' }, `Could not read the world: ${(e as Error).message}`),
        h('button', { type: 'button', class: 'primary', onclick: () => { root.replaceChildren(header, h('p', { class: 'muted' }, 'Trying again...')); void load(); } }, 'Try again'),
      );
      return;
    }

    const parts: Array<Node | null> = [header];
    // The page and the contract must run the same rules; if the contract names a different hash, say so.
    const rules = rulesInfo();
    const chainRules = await coreVersion().catch(() => null);
    if (chainRules !== null && (chainRules.rules_sha256 !== rules.rulesHash || chainRules.core_sha256 !== rules.coreHash)) {
      parts.push(h('div', { class: 'banner warn', role: 'alert' },
        `This page runs rules ${rules.rulesHash.slice(0, 12)} but the contract reports ${chainRules.rules_sha256.slice(0, 12)}. Results shown here may differ from what the contract records.`));
    }
    if (flash) {
      parts.push(h('div', { class: `banner ${flash.kind}`, role: 'status' }, flash.text,
        ...(flash.links ?? []).flatMap((l) => [' ', h('a', { href: l.url, target: '_blank', rel: 'noopener' }, l.label)])));
    }

    if (active) {
      const saved = loadDive(session.accountId, active.dive_id);
      parts.push(h('section', { class: 'card' },
        h('h2', {}, 'A dive of yours is still open'),
        h('p', {}, saved
          ? `You are ${saved.moves.length} moves in. Everything you took down stays locked until the dive is recorded.`
          : 'This browser has no record of those moves, so the dive restarts from its first step. Your locked items stay locked until a dive is recorded.'),
        h('p', { class: 'muted small' }, saved
          ? 'Or end it here: the contract records the moves so far, a death included.'
          : 'Or end it here: with no moves recorded you come back with what you took down and nothing more.'),
        h('div', { class: 'row' },
          h('button', { class: 'primary', type: 'button', onclick: () => actions.onContinue(active, saved) }, saved ? 'Continue the dive' : 'Restart the dive'),
          h('button', { type: 'button', onclick: () => actions.onEnd(active, saved) }, 'End the dive here'))));
    } else {
      const free = available(me);
      const picks = new Map<string, number>();
      const stepper = (token: string, max: number): HTMLElement => {
        const value = h('strong', {}, '0');
        const set = (n: number) => { picks.set(token, n); value.textContent = String(n); };
        return h('label', {},
          h('span', {}, `${itemLabel(token)} `, h('span', { class: 'muted small' }, `(${max})`)),
          h('span', { class: 'stepper' },
            h('button', { type: 'button', 'aria-label': `Fewer ${itemLabel(token)}`, onclick: () => set(Math.max(0, (picks.get(token) ?? 0) - 1)) }, '−'),
            value,
            h('button', { type: 'button', 'aria-label': `More ${itemLabel(token)}`, onclick: () => set(Math.min(max, (picks.get(token) ?? 0) + 1)) }, '+')));
      };
      parts.push(h('section', { class: 'card' },
        h('h2', {}, 'Choose what to carry down'),
        h('p', { class: 'muted small' }, 'What you carry is what you risk. If you die, it all stays in your grave for the next player.'),
        free.size
          ? h('div', { class: 'loadout' }, ...[...free].map(([t, n]) => stepper(t, n)))
          : h('p', {}, 'Your stash is empty. You can still dive with nothing to lose, and keep whatever you find.'),
        h('div', { class: 'row', style: 'margin-top:12px' },
          h('button', { class: 'primary', type: 'button', onclick: () => actions.onDive([...picks].filter(([, n]) => n > 0)) }, 'Dive'))));
    }

    parts.push(gravesCard(graves));
    parts.push(h('section', { class: 'card' },
      h('h2', {}, `Today is day ${day}`),
      h('div', { class: 'chips' },
        h('span', { class: 'chip' }, `Dives ${me.dives}`),
        h('span', { class: 'chip' }, `Deaths ${me.deaths}`),
        h('span', { class: 'chip' }, `Deepest ${me.deepest}`)),
      h('p', { class: 'muted small', style: 'margin-top:8px' }, 'Everyone gets the same dungeon on the same day.')));
    root.replaceChildren(...parts.filter((p): p is Node => p !== null));
  }

  void load();
  return root;
}
