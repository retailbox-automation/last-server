// The game rules in the browser: the Rust crate the contract runs, compiled to wasm by scripts/build-wasm.sh.
// Types mirror the JSON that ls-core serializes (DiveView, StepEvents, DiveOutcome, DiveEntry).

import initWasm, * as wasm from './wasm/ls_wasm.js';

export type ItemAmount = [itemId: number, amount: number];

export interface FloorItem { x: number; y: number; item: number; amount: number }
export interface MonsterView { id: number; x: number; y: number; hp: number }
export interface GraveMarker { id: number; x: number; y: number; looted: boolean; items: ItemAmount[] }

export interface DiveView {
  depth: number;
  turn: number;
  width: number;
  height: number;
  /** Row-major: tiles[y * width + x]. */
  tiles: number[];
  x: number;
  y: number;
  hp: number;
  max_hp: number;
  monsters: MonsterView[];
  items: FloorItem[];
  graves: GraveMarker[];
  inventory: ItemAmount[];
  blade_durability: number;
  kills: number;
  moves_applied: number;
  over: boolean;
  died: boolean;
}

export type GameEvent =
  | { kind: 'moved'; x: number; y: number }
  | { kind: 'bumped'; x: number; y: number }
  | { kind: 'attacked'; monster: number; damage: number; killed: boolean }
  | { kind: 'dropped'; x: number; y: number; item: number; amount: number }
  | { kind: 'hurt'; monster: number; damage: number }
  | { kind: 'missed'; monster: number }
  | { kind: 'picked'; item: number; amount: number }
  | { kind: 'grave_looted'; grave_id: number }
  | { kind: 'blade_broke' }
  | { kind: 'potion_drunk'; healed: number }
  | { kind: 'door_opened'; x: number; y: number }
  | { kind: 'door_locked'; x: number; y: number }
  | { kind: 'descended'; depth: number }
  | { kind: 'regenerated' }
  | { kind: 'spawned'; monster: number; x: number; y: number }
  | { kind: 'died'; x: number; y: number };

export interface StepEvents { applied: boolean; events: GameEvent[] }

export interface DiveOutcome {
  final_depth: number;
  died: boolean;
  moves_applied: number;
  picked: ItemAmount[];
  looted_graves: number[];
  dropped_into_grave: ItemAmount[];
  burned: ItemAmount[];
  death_pos: [number, number] | null;
  hp_left: number;
  kills: number;
  blades_broken: number;
  potions_used: number;
  keys_used: number;
  vault_opened: boolean;
}

export interface Grave { id: number; depth: number; x: number; y: number; items: ItemAmount[] }
export interface DiveEntry { start_depth: number; inventory: ItemAmount[]; graves: Grave[] }

export const MOVE = { up: 0, down: 1, left: 2, right: 3, wait: 4 } as const;
export const TILE = { wall: 0, floor: 1, stairs: 2 } as const;

let ready: Promise<void> | null = null;

/** Loads and instantiates the wasm module once. */
export function loadRules(): Promise<void> {
  ready ??= initWasm().then(() => undefined);
  return ready;
}

export interface RulesInfo {
  rulesHash: string;
  coreHash: string;
  version: string;
  maxMoves: number;
  /** item id (from the rules) -> name; the name is the NEP-245 token id. */
  itemNames: Map<number, string>;
}

export function rulesInfo(): RulesInfo {
  const pairs = JSON.parse(wasm.item_names()) as Array<[number, string]>;
  return {
    rulesHash: wasm.rules_hash(),
    coreHash: wasm.core_hash(),
    version: wasm.core_version(),
    maxMoves: wasm.max_moves(),
    itemNames: new Map(pairs),
  };
}

/** One dive in progress; owns the wasm object, so call `free()` when done. */
export class DiveSession {
  private readonly sim: wasm.Sim;

  constructor(seed: bigint, entry: DiveEntry) {
    this.sim = new wasm.Sim(seed, JSON.stringify(entry));
  }

  step(mv: number): StepEvents {
    return JSON.parse(this.sim.step(mv)) as StepEvents;
  }

  view(): DiveView {
    return JSON.parse(this.sim.view()) as DiveView;
  }

  isOver(): boolean {
    return this.sim.is_over();
  }

  outcome(): DiveOutcome {
    return JSON.parse(this.sim.outcome()) as DiveOutcome;
  }

  free(): void {
    this.sim.free();
  }
}
