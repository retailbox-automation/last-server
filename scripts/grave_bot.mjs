// Move-log generators for the e2e grave path, built on the browser's wasm rules (web/src/wasm).
// Not part of the game: they play the same stepper a player would, so the contract replays them.
//   dieWithLoot: pick something up on the first depth, then stand next to monsters until killed.
//   lootGrave:   walk to one grave (taking stairs down to its depth if needed), step on it, walk
//                back to the way up of that depth and climb out (MOVE_SURFACE): only that banks loot.
//   SURFACE_NOW: a one-move log that climbs straight out (a dive starts on the way up): a survived dive.
// Each returns { moves, outcome } from a local run, or null if the plan did not work out on this map.
// Rules since core-v1-fix: a log that ends inside the dungeon is a quit = a death with a grave, and
// one dive loots at most GRAVES_LOOTED_PER_DIVE (50) graves.

const MAX_MOVES = 4000;
const WAIT = 4;
const SURFACE = 6;
export const SURFACE_NOW = Uint8Array.of(SURFACE);
const FLOOR = 1;
const STAIRS = 2;
// [move byte, dx, dy] in ls-core's numbering: up, down, left, right.
const DIRS = [[0, 0, -1], [1, 0, 1], [2, -1, 0], [3, 1, 0]];

/**
 * First move of a shortest path from the player to any cell `isGoal` accepts, through floor (and
 * stairs only when the stairs are the goal), never through a monster. null when unreachable.
 */
function firstStep(v, isGoal, avoid = () => false) {
  const { width: w, height: h, tiles } = v;
  const monster = new Set(v.monsters.map((m) => m.y * w + m.x));
  const start = v.y * w + v.x;
  const first = new Map([[start, -1]]);
  const queue = [start];
  for (let i = 0; i < queue.length; i++) {
    const c = queue[i];
    const x = c % w;
    const y = (c - x) / w;
    if (c !== start && isGoal(x, y)) return first.get(c);
    for (const [mv, dx, dy] of DIRS) {
      const nx = x + dx;
      const ny = y + dy;
      if (nx < 0 || ny < 0 || nx >= w || ny >= h) continue;
      const n = ny * w + nx;
      if (first.has(n) || monster.has(n)) continue;
      const t = tiles[n];
      if (!(t === FLOOR || (t === STAIRS && isGoal(nx, ny)))) continue;
      if (!isGoal(nx, ny) && avoid(nx, ny)) continue;
      first.set(n, c === start ? mv : first.get(c));
      queue.push(n);
    }
  }
  return null;
}

const nextToMonster = (v) => (x, y) => v.monsters.some((m) => Math.abs(m.x - x) + Math.abs(m.y - y) === 1);

function run(w, seed, entry, choose) {
  const sim = new w.Sim(BigInt(seed), JSON.stringify(entry));
  const moves = [];
  try {
    while (moves.length < MAX_MOVES) {
      const v = JSON.parse(sim.view());
      if (v.over) break;
      const mv = choose(v, JSON.parse(sim.outcome()));
      const step = JSON.parse(sim.step(mv));
      if (!step.applied) break;
      moves.push(mv);
    }
    return { moves: Uint8Array.from(moves), outcome: JSON.parse(sim.outcome()) };
  } finally {
    sim.free();
  }
}

/** Next move toward the nearest item on this depth, keeping clear of monsters while unarmed. */
function towardItem(v) {
  const items = new Set(v.items.map((it) => it.y * v.width + it.x));
  const goal = (x, y) => items.has(y * v.width + x);
  return firstStep(v, goal, nextToMonster(v)) ?? firstStep(v, goal) ?? WAIT;
}

export function dieWithLoot(w, seed, entry) {
  const r = run(w, seed, entry, (v) => {
    if (v.inventory.length === 0) return towardItem(v);
    if (nextToMonster(v)(v.x, v.y)) return WAIT;
    return firstStep(v, nextToMonster(v)) ?? WAIT;
  });
  // Killed, not quit: the grave path should exercise a real death (a quit at the move cap would also leave a grave).
  return r.outcome.died && !r.outcome.quit && r.outcome.dropped_into_grave.length > 0 ? r : null;
}

export function lootGrave(w, seed, entry, graveId) {
  const r = run(w, seed, entry, (v, o) => {
    if (o.looted_graves.includes(graveId)) {
      // Take it home: back to where this depth was entered, then up.
      if (v.x === v.exit_x && v.y === v.exit_y) return SURFACE;
      const exit = (x, y) => x === v.exit_x && y === v.exit_y;
      return firstStep(v, exit, nextToMonster(v)) ?? firstStep(v, exit) ?? WAIT;
    }
    const target = v.graves.find((g) => g.id === graveId && !g.looted);
    const avoid = nextToMonster(v);
    if (target) {
      const goal = (x, y) => x === target.x && y === target.y;
      return firstStep(v, goal, avoid) ?? firstStep(v, goal) ?? WAIT;
    }
    // The grave is deeper: take the stairs.
    const stairs = (x, y) => v.tiles[y * v.width + x] === STAIRS;
    return firstStep(v, stairs, avoid) ?? firstStep(v, stairs) ?? WAIT;
  });
  return r.outcome.returned && r.outcome.looted_graves.includes(graveId) ? r : null;
}
