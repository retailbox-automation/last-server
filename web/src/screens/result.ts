import { explorerTxUrl } from '../chain.ts';
import { h } from '../dom.ts';
import { describeItems, describeTokens, shortName } from '../items.ts';
import type { DiveOutcome, GameEvent } from '../sim.ts';
import type { GraveView } from '../world.ts';
import type { CallResult } from '../tx.ts';
import type { DiveFinish } from './dive.ts';

export interface ResultInput {
  /** How the dive was started; null if unknown. */
  start: { txHash: string; via: 'relayer' | 'direct' | 'state' } | null;
  settle: CallResult;
  /** The contract's own account of the dive, if it returned one. */
  chain: DiveOutcome | null;
  finish: DiveFinish;
  graves: GraveView[];
}

/** The fields a player can see; a difference means the chain saw a different world than the page did. */
function digest(o: DiveOutcome): string {
  return JSON.stringify([o.died, o.returned, o.quit, o.final_depth, o.picked, o.looted_graves, o.dropped_into_grave, o.keys_used, o.blades_broken]);
}

function whyLines(o: DiveOutcome, history: GameEvent[], graves: GraveView[]): string[] {
  const byId = new Map(graves.map((g) => [Number(g.id), g]));
  const lines: string[] = [];
  lines.push(`You reached depth ${o.final_depth} in ${o.moves_applied} moves and defeated ${o.kills} monster${o.kills === 1 ? '' : 's'}.`);
  for (const id of o.looted_graves) {
    const g = byId.get(id);
    lines.push(g ? `Grave met: you emptied ${shortName(g.owner_id)}'s grave at depth ${g.depth} and took ${describeTokens(g.items)}.` : `Grave met: you emptied grave #${id}.`);
  }
  const lockedHits = history.filter((e) => e.kind === 'door_locked').length;
  if (o.keys_used > 0) lines.push(`Key used: ${o.keys_used} blue key burned to open a door${o.vault_opened ? '; the vault behind it was yours' : ''}.`);
  else if (lockedHits > 0) lines.push('A locked door stopped you: it needs a blue key.');
  if (o.blades_broken > 0) lines.push(`Broken: ${o.blades_broken} blade${o.blades_broken === 1 ? '' : 's'} wore out and ${o.blades_broken === 1 ? 'was' : 'were'} burned.`);
  if (o.potions_used > 0) lines.push(`Used up: ${o.potions_used} potion${o.potions_used === 1 ? '' : 's'}.`);
  if (o.picked.length) lines.push(`Found down there: ${describeItems(o.picked)}.`);
  const where = `depth ${o.final_depth}${o.death_pos ? ` (${o.death_pos[0]}, ${o.death_pos[1]})` : ''}`;
  const grave = o.dropped_into_grave.length ? `Your grave there now holds ${describeItems(o.dropped_into_grave)}.` : 'You carried nothing, so no grave was left.';
  if (o.quit) {
    lines.push(`The dive ended inside the dungeon, not on the way up, so it counts as a death at ${where}. ${grave}`);
    if (o.burned.length) lines.push(`The death toll burned ${describeItems(o.burned)}.`);
  } else if (o.died) {
    lines.push(`You fell at ${where}. ${grave}`);
    if (o.burned.length) lines.push(`The death toll burned ${describeItems(o.burned)}.`);
  } else {
    lines.push('You came back alive: everything you carried and found is in your stash.');
  }
  return lines;
}

const ending = (o: DiveOutcome): string => (o.returned ? 'climbed out' : o.quit ? 'gave up inside the dungeon' : 'were killed');

/**
 * The outcome the screen shows: the contract's, whenever the settle call returned one. The page's
 * own run can differ legitimately, e.g. another player emptied a grave between start and settle and
 * the contract's replay simply does not find it. `notes` says why, grave by grave.
 */
export function outcomeToShow(chain: DiveOutcome | null, local: DiveOutcome, graves: GraveView[]): { shown: DiveOutcome; source: string; notes: string[] } {
  if (chain === null) {
    return {
      shown: local,
      source: 'from your local run: the reply from the chain was lost, and the record on the chain is the one that counts',
      notes: [],
    };
  }
  const notes: string[] = [];
  if (digest(chain) !== digest(local)) {
    const owner = (id: number) => {
      const g = graves.find((x) => Number(x.id) === id);
      return g ? `${shortName(g.owner_id)}'s grave (#${id})` : `Grave #${id}`;
    };
    for (const id of local.looted_graves.filter((g) => !chain.looted_graves.includes(g))) {
      notes.push(`${owner(id)} was emptied by another player before your dive was recorded, so the contract's replay did not find it. The result below is the contract's.`);
    }
    for (const id of chain.looted_graves.filter((g) => !local.looted_graves.includes(g))) {
      notes.push(`The contract's replay found ${owner(id)}, which your page did not see (it appeared after your dive started).`);
    }
    if (ending(chain) !== ending(local)) {
      notes.push(`Your page says you ${ending(local)}; the contract recorded that you ${ending(chain)}. The contract's record is the one that counts.`);
    }
    if (notes.length === 0) {
      notes.push('The contract saw the world differently than your page did. The contract result below is the one that counts.');
    }
  }
  return { shown: chain, source: 'recorded by the contract', notes };
}

function banner(o: DiveOutcome): string {
  if (o.returned) return 'Dive recorded. You climbed out with everything you carried.';
  if (o.quit) return 'Dive recorded as given up. That counts as a death: your grave is now part of the world.';
  return 'Dive recorded. You died, and your grave is now part of the world.';
}

export function resultScreen(input: ResultInput, onBack: () => void): HTMLElement {
  const { shown, source, notes } = outcomeToShow(input.chain, input.finish.local, input.graves);

  const link = (label: string, tx: string) => tx
    ? h('li', {}, `${label}: `, h('a', { href: explorerTxUrl(tx), target: '_blank', rel: 'noopener' }, 'view transaction'), ' ', h('span', { class: 'mono muted' }, tx.slice(0, 10) + '...'))
    : h('li', {}, `${label}: on the chain (transaction id unknown)`);
  const via = (c: { via: 'relayer' | 'direct' | 'state' }) =>
    c.via === 'relayer' ? 'sent through the studio relayer, you paid nothing'
      : c.via === 'direct' ? 'sent directly with your own key (the relayer was not available)'
        : 'confirmed by reading the contract (the reply to the first attempt was lost)';

  return h('div', {},
    h('div', { class: `banner ${shown.returned ? 'good' : 'warn'}`, role: 'status' }, banner(shown)),
    h('section', { class: 'card' },
      h('h2', {}, 'Why it went this way'),
      h('p', { class: 'muted small' }, `Result ${source}.`),
      ...notes.map((n) => h('div', { class: 'banner warn' }, n)),
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
