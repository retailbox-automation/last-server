// The world contract as the page sees it: named views and the two calls a player makes.
// All shapes follow SPEC-CORE-V1 section "A. Contract"; if the contract's JSON changes, this is the file to edit.

import { toBase64 } from './bytes.ts';
import { view } from './chain.ts';
import type { Session } from './keys.ts';
import { callWorld } from './tx.ts';
import type { CallResult } from './tx.ts';
import type { DiveEntry, DiveOutcome, Grave, ItemAmount } from './sim.ts';

export interface DiveTicket {
  dive_id: string;
  day: number;
  seed: string;
  start_depth: number;
}

/** One row of the `graves(depth)` view. */
export interface GraveView {
  id: string | number;
  owner_id: string;
  depth: number;
  x: number;
  y: number;
  items: ItemAmount[];
}

export interface PlayerView {
  /** token id -> amount (strings, u128) the player owns. */
  stash: Array<[string, string]>;
  /** Part of the stash currently locked inside an active dive. */
  locked: Array<[string, string]>;
  dives: number;
  deaths: number;
  deepest: number;
}

export const currentDay = () => view<number>('current_day');
export const activeDive = (accountId: string) => view<DiveTicket | null>('active_dive', { account_id: accountId });
export const diveEntry = (accountId: string) => view<DiveEntry>('dive_entry', { account_id: accountId });
export const gravesSummary = () => view<Array<[number, number]>>('graves_summary');
export const gravesAt = (depth: number) => view<GraveView[]>('graves', { depth });
export const player = (accountId: string) => view<PlayerView>('player', { account_id: accountId });
export const coreVersion = () => view<string>('core_version');

/** Every grave of every depth that has one, in the order the contract lists them per depth. */
export async function allGraves(): Promise<GraveView[]> {
  const depths = (await gravesSummary()).filter(([, n]) => n > 0).map(([d]) => d);
  const perDepth = await Promise.all(depths.map((d) => gravesAt(d)));
  return perDepth.flat();
}

export function toSimGrave(g: GraveView): Grave {
  return { id: Number(g.id), depth: g.depth, x: g.x, y: g.y, items: g.items };
}

/** loadout: token id -> amount. */
export async function startDive(
  session: Session,
  loadout: Array<[token: string, amount: number]>,
): Promise<{ ticket: DiveTicket; call: CallResult }> {
  const call = await callWorld(session, 'start_dive', { loadout: loadout.map(([t, n]) => [t, String(n)]) });
  const ticket = call.result as DiveTicket | null;
  if (!ticket || ticket.dive_id === undefined) {
    // The relayer or RPC returned no value; read the ticket back from the chain.
    const t = await activeDive(session.accountId);
    if (!t) throw new Error('The dive was sent but the contract has no active dive for you yet. Try again in a moment.');
    return { ticket: t, call };
  }
  return { ticket, call };
}

export async function settleDive(
  session: Session,
  diveId: string,
  moves: Uint8Array,
): Promise<{ outcome: DiveOutcome | null; call: CallResult }> {
  const call = await callWorld(session, 'settle_dive', { dive_id: diveId, moves: toBase64(moves) });
  return { outcome: (call.result as DiveOutcome | null) ?? null, call };
}
