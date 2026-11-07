import type { ItemAmount } from './sim.ts';
import { rulesInfo } from './sim.ts';

/** Display names; anything the rules add later falls back to its token id. */
const LABELS: Record<string, string> = {
  gold: 'Gold',
  blade: 'Blade',
  plate: 'Plate armor',
  potion: 'Potion',
  blue_key: 'Blue Key',
  gem: 'Gem',
};

export const ITEM_COLOR: Record<string, string> = {
  gold: '#f2c94c',
  blade: '#c9d3e6',
  plate: '#8896ad',
  potion: '#e5484d',
  blue_key: '#3b82f6',
  gem: '#2dd4bf',
};

export function itemLabel(token: string): string {
  return LABELS[token] ?? token.replace(/_/g, ' ');
}

/** Token id (NEP-245 id) for a rules item id. */
export function tokenOf(id: number): string {
  return rulesInfo().itemNames.get(id) ?? `item-${id}`;
}

export function describeItems(items: ItemAmount[]): string {
  if (items.length === 0) return 'nothing';
  return items.map(([id, n]) => `${n} ${itemLabel(tokenOf(id))}`).join(', ');
}

/** `mara.lastserver.testnet` -> `mara` */
export function shortName(accountId: string): string {
  const dot = accountId.indexOf('.');
  return dot > 0 ? accountId.slice(0, dot) : accountId;
}
