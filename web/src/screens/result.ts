import { explorerTxUrl } from '../chain.ts';
import { h } from '../dom.ts';
import { describeItems, shortName } from '../items.ts';
import type { DiveOutcome, GameEvent } from '../sim.ts';
import type { GraveView } from '../world.ts';
import type { CallResult } from '../tx.ts';
import type { DiveFinish } from './dive.ts';

export interface ResultInput {
  /** How the dive was started; null if unknown. */
  start: { txHash: string; via: 'relayer' | 'direct' } | null;
  settle: CallResult;
  /** The contract's own account of the dive, if it returned one. */
  chain: DiveOutcome | null;
  finish: DiveFinish;
  graves: GraveView[];
}

/** The fields a player can see; a difference means the chain saw a different world than the page did. */
function digest(o: DiveOutcome): string {
  return JSON.stringify([o.died, o.final_depth, o.picked, o.looted_graves, o.dropped_into_grave, o.keys_used, o.blades_broken]);
}

function whyLines(o: DiveOutcome, history: GameEvent[], graves: GraveView[]): string[] {
  const byId = new Map(graves.map((g) => [Number(g.id), g]));
  const lines: string[] = [];
  lines.push(`You reached depth ${o.final_depth} in ${o.moves_applied} moves and defeated ${o.kills} monster${o.kills === 1 ? '' : 's'}.`);
  for (const id of o.looted_graves) {
    const g = byId.get(id);
    lines.push(g ? `Grave met: you emptied ${shortName(g.owner_id)}'s grave at depth ${g.depth} and took ${describeItems(g.items)}.` : `Grave met: you emptied grave #${id}.`);
  }
  const lockedHits = history.filter((e) => e.kind === 'door_locked').length;
  if (o.keys_used > 0) lines.push(`Key used: ${o.keys_used} blue key burned to open a door${o.vault_opened ? '; the vault behind it was yours' : ''}.`);
  else if (lockedHits > 0) lines.push('A locked door stopped you: it needs a blue key.');
  if (o.blades_broken > 0) lines.push(`Broken: ${o.blades_broken} blade${o.blades_broken === 1 ? '' : 's'} wore out and ${o.blades_broken === 1 ? 'was' : 'were'} burned.`);
  if (o.potions_used > 0) lines.push(`Used up: ${o.potions_used} potion${o.potions_used === 1 ? '' : 's'}.`);
  if (o.picked.length) lines.push(`Found down there: ${describeItems(o.picked)}.`);
  if (o.died) {
    lines.push(`You fell at depth ${o.final_depth}${o.death_pos ? ` (${o.death_pos[0]}, ${o.death_pos[1]})` : ''}. Your grave now holds ${describeItems(o.dropped_into_grave)}.`);
    if (o.burned.length) lines.push(`The death toll burned ${describeItems(o.burned)}.`);
  } else {
    lines.push('You came back alive: everything you carried and found is in your stash.');
  }
  return lines;
}

export function resultScreen(input: ResultInput, onBack: () => void): HTMLElement {
  const shown = input.chain ?? input.finish.local;
  const source = input.chain ? 'recorded by the contract' : 'from your local run (the contract returned no details)';
  const differs = input.chain !== null && digest(input.chain) !== digest(input.finish.local);

  const link = (label: string, tx: string) => h('li', {}, `${label}: `, h('a', { href: explorerTxUrl(tx), target: '_blank', rel: 'noopener' }, 'view transaction'), ' ', h('span', { class: 'mono muted' }, tx.slice(0, 10) + '...'));
  const via = (c: { via: 'relayer' | 'direct' }) => (c.via === 'relayer' ? 'sent through the studio relayer, you paid nothing' : 'sent directly with your own key (the relayer was not available)');

  return h('div', {},
    h('div', { class: `banner ${shown.died ? 'warn' : 'good'}`, role: 'status' }, shown.died ? 'Dive recorded. You died, and your grave is now part of the world.' : 'Dive recorded. You made it back.'),
    h('section', { class: 'card' },
      h('h2', {}, 'Why it went this way'),
      h('p', { class: 'muted small' }, `Result ${source}.`),
      differs ? h('div', { class: 'banner warn' }, 'The contract saw the world differently than your page did (for example someone else reached a grave first). The contract result below is the one that counts.') : null,
      h('ul', { class: 'why' }, ...whyLines(shown, input.finish.history, input.graves).map((l) => h('li', {}, l))),
    ),
    h('section', { class: 'card' },
      h('h2', {}, 'On the chain'),
      h('ul', {},
        input.start ? link('Dive started', input.start.txHash) : null,
        link('Dive settled', input.settle.txHash),
      ),
      h('p', { class: 'muted small' }, `${input.start ? `Start ${via(input.start)}. ` : ''}Settle ${via(input.settle)}.`),
    ),
    h('button', { class: 'primary', type: 'button', onclick: onBack }, 'Back to the surface'),
  );
}
