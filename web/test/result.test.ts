// The result screen shows the contract's outcome, and says why it differs from the page's own run.
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { outcomeToShow } from '../src/screens/result.ts';
import type { DiveOutcome } from '../src/sim.ts';
import type { GraveView } from '../src/world.ts';

const base: DiveOutcome = {
  final_depth: 2, died: false, returned: true, quit: false, moves_applied: 40, picked: [], looted_graves: [],
  dropped_into_grave: [], burned: [], death_pos: null, hp_left: 5, kills: 1, blades_broken: 0, potions_used: 0,
  keys_used: 0, vault_opened: false,
};
const graves: GraveView[] = [
  { id: 5, owner_id: 'mara.lastserver.testnet', depth: 2, x: 4, y: 4, items: [['plate', '1']], day: 1, dive_id: '9' },
];

describe('outcomeToShow', () => {
  it('local run survived thanks to a grave someone else emptied first; the contract says killed: the contract wins', () => {
    const local: DiveOutcome = { ...base, looted_graves: [5], picked: [] };
    const chain: DiveOutcome = { ...base, returned: false, died: true, looted_graves: [], death_pos: [6, 7], dropped_into_grave: [[1, 3]] };
    const r = outcomeToShow(chain, local, graves);
    assert.equal(r.shown, chain);
    assert.equal(r.source, 'recorded by the contract');
    assert.ok(r.notes.some((n) => n.includes("mara's grave (#5)") && n.includes('another player')), r.notes.join(' | '));
    assert.ok(r.notes.some((n) => n.includes('climbed out') && n.includes('were killed')), r.notes.join(' | '));
  });

  it('same outcome on both sides: no notes', () => {
    const r = outcomeToShow({ ...base }, { ...base }, graves);
    assert.deepEqual(r.notes, []);
  });

  it('no outcome from the chain (reply lost): the local run, labelled as such', () => {
    const r = outcomeToShow(null, base, graves);
    assert.equal(r.shown, base);
    assert.match(r.source, /local run/);
  });

  it('a quit is named as giving up, not as a kill', () => {
    const chain: DiveOutcome = { ...base, returned: false, quit: true, died: true, death_pos: [1, 1] };
    const r = outcomeToShow(chain, base, graves);
    assert.ok(r.notes.some((n) => n.includes('gave up inside the dungeon')));
  });
});
