// A dive in progress survives a reload: the move log and the entry it started from are kept in
// localStorage, so closing the tab does not turn a dive into a death.

import { fromBase64, toBase64 } from './bytes.ts';
import type { DiveEntry } from './sim.ts';
import type { DiveTicket } from './world.ts';

export interface StartInfo {
  txHash: string;
  via: 'relayer' | 'direct';
}

/** What a dive needs to be replayed: its ticket, the state it started from, and how it was started. */
export interface DiveBase {
  ticket: DiveTicket;
  entry: DiveEntry;
  start: StartInfo | null;
}

export interface SavedDive extends DiveBase {
  /** Base64 of the move bytes so far. */
  moves: string;
}

const key = (accountId: string) => `lastserver.dive.v1.${accountId}`;

export function saveDive(accountId: string, base: DiveBase, moves: Uint8Array): void {
  try {
    const saved: SavedDive = { ...base, moves: toBase64(moves) };
    localStorage.setItem(key(accountId), JSON.stringify(saved));
  } catch {
    // Storage blocked: the dive still works, it just cannot be resumed after a reload.
  }
}

/** The saved dive for this account, only if it belongs to the contract's active dive. */
export function loadDive(accountId: string, activeDiveId: string): { saved: SavedDive; moves: Uint8Array } | null {
  try {
    const raw = localStorage.getItem(key(accountId));
    if (!raw) return null;
    const saved = JSON.parse(raw) as SavedDive;
    if (saved.ticket?.dive_id !== activeDiveId) return null;
    return { saved, moves: fromBase64(saved.moves) };
  } catch {
    return null;
  }
}

export function clearDive(accountId: string): void {
  try {
    localStorage.removeItem(key(accountId));
  } catch {
    // Nothing to clear.
  }
}
