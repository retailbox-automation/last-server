// Reads from the world contract. The whole RPC surface of the page is this file plus tx.ts,
// so pointing the game at another endpoint or contract is a configuration change.

import { JsonRpcProvider } from 'near-api-js';
import { AccountDoesNotExistError } from 'near-api-js/rpc-errors';
import { config } from './config.ts';

export const provider = new JsonRpcProvider({ url: config.rpcUrl });

/** Calls a view method of the world contract and returns its parsed JSON result. */
export async function view<T>(method: string, args: Record<string, unknown> = {}): Promise<T> {
  const r = await provider.callFunction<any>({
    contractId: config.worldContractId,
    method,
    args,
    blockQuery: { finality: 'optimistic' },
  });
  return r as T;
}

export function explorerTxUrl(txHash: string): string {
  return config.explorerTxBase + txHash;
}

export async function accountExists(accountId: string): Promise<boolean> {
  try {
    await provider.viewAccount({ accountId, blockQuery: { finality: 'optimistic' } });
    return true;
  } catch (e) {
    // instanceof, not the error's name: a minified build renames classes.
    if (e instanceof AccountDoesNotExistError) return false;
    throw e;
  }
}
