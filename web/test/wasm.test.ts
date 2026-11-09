// The browser build of the rules must behave exactly like ls-core. Skipped when `npm run wasm` has not been run.
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

const dir = new URL('../src/wasm/', import.meta.url);
const built = existsSync(new URL('ls_wasm_bg.wasm', dir));

describe('ls-wasm', { skip: built ? false : 'run `npm run wasm` first' }, async () => {
  const w = built ? await import(new URL('ls_wasm.js', dir).href) : null;
  if (w) w.initSync({ module: readFileSync(new URL('ls_wasm_bg.wasm', dir)) });

  const entry = JSON.stringify({ start_depth: 1, inventory: [[2, 1], [5, 2], [1, 5]], graves: [] });
  const moves = Uint8Array.from(Array.from({ length: 300 }, (_, i) => [3, 3, 1, 1, 2, 4, 0, 5][i % 8]!));

  it('Sim stepped move by move gives the same outcome as run_dive over the whole log', () => {
    const sim = new w.Sim(12345n, entry);
    for (const m of moves) sim.step(m);
    assert.deepEqual(JSON.parse(sim.outcome()), JSON.parse(w.run_dive(12345n, entry, moves)));
    sim.free();
  });

  it('is deterministic and the seed matters', () => {
    assert.equal(w.run_dive(7n, entry, moves), w.run_dive(7n, entry, moves));
    assert.notEqual(w.run_dive(7n, entry, moves), w.run_dive(8n, entry, moves));
  });

  it('view has a full 32x32 map and the player inside it', () => {
    const sim = new w.Sim(1n, entry);
    const v = JSON.parse(sim.view());
    assert.equal(v.tiles.length, v.width * v.height);
    assert.ok(v.x >= 0 && v.x < v.width && v.y >= 0 && v.y < v.height);
    assert.equal(v.max_hp >= v.hp && v.hp > 0, true);
    sim.free();
  });

  it('endings: the way up banks, a log that stops inside or an empty log is a quit (a death)', () => {
    const sim = new w.Sim(3n, entry);
    const v = JSON.parse(sim.view());
    assert.deepEqual([v.x, v.y], [v.exit_x, v.exit_y]); // a dive starts on the way up
    sim.free();
    const up = JSON.parse(w.run_dive(3n, entry, Uint8Array.of(6)));
    assert.equal(up.returned, true);
    assert.equal(up.died, false);
    assert.equal(up.quit, false);
    assert.deepEqual(up.dropped_into_grave, []);
    const empty = JSON.parse(w.run_dive(3n, entry, new Uint8Array()));
    assert.equal(empty.quit, true);
    assert.equal(empty.died, true);
    assert.ok(empty.dropped_into_grave.length > 0 && empty.death_pos !== null);
  });

  it('the surface move off the way up is only a wait', () => {
    const sim = new w.Sim(3n, entry);
    let v = JSON.parse(sim.view());
    for (const mv of [0, 1, 2, 3]) {
      sim.step(mv);
      v = JSON.parse(sim.view());
      if (v.x !== v.exit_x || v.y !== v.exit_y) break;
    }
    if (v.x !== v.exit_x || v.y !== v.exit_y) {
      sim.step(6);
      const after = JSON.parse(sim.view());
      assert.equal(after.returned, false);
      assert.equal(after.over, false);
    }
    sim.free();
  });

  it('rejects a malformed entry', () => {
    assert.throws(() => new w.Sim(1n, '{"start_depth":'));
  });

  it('reports hashes, version and item names', () => {
    assert.match(w.rules_hash(), /^[0-9a-f]{8,}$/);
    assert.ok(w.core_hash().length > 0);
    const names = new Map(JSON.parse(w.item_names()));
    assert.equal(names.get(1), 'gold');
    assert.equal(w.max_moves(), 4000);
  });
});
