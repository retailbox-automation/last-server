// Sends a call to the world contract as the player: through the relayer as a NEP-366 signed
// delegate (the player pays nothing and sees no prompt), or, if the relayer is down or overloaded,
// directly with the player's own key and 100 Tgas.
//
// A relayer that failed may still have sent the call (it timed out waiting, or answered 504), and
// a call the contract refused may have been refused only because an earlier attempt of the same
// call already went through. In both cases the caller's `alreadyDone` reads the contract state
// first; the call is sent again only if that state shows it did not happen.

import {
  Account,
  KeyPair,
  KeyPairSigner,
  actions,
  encodeSignedDelegate,
  getTransactionLastResult,
} from 'near-api-js';
import { toBase64 } from './bytes.ts';
import { provider } from './chain.ts';
import { config } from './config.ts';
import type { Session } from './keys.ts';
import { RelayerRefused, RelayerUnavailable, relay } from './relayer.ts';

export const GAS_100_TGAS = 100_000_000_000_000n;
/** A signed delegate expires after this many blocks (about 2 minutes), so a stuck one cannot be replayed later. */
const DELEGATE_TTL_BLOCKS = 100;

export interface CallResult {
  /** Empty when the call was confirmed from the contract state and its transaction is unknown. */
  txHash: string;
  /** `state`: an earlier attempt went through; confirmed by reading the contract, not by a reply. */
  via: 'relayer' | 'direct' | 'state';
  /** Parsed JSON return value of the contract method (null when confirmed from state). */
  result: unknown;
}

/** Reads the contract and returns a result if the call has already taken effect, else null. */
export type AlreadyDone = (txHash: string | undefined) => Promise<CallResult | null>;

/** The contract rejected or failed the call (panic message in `message`). */
export class CallFailed extends Error {
  readonly txHash: string | undefined;
  constructor(message: string, txHash?: string) {
    super(message);
    this.name = 'CallFailed';
    this.txHash = txHash;
  }
}

function playerAccount(session: Session): Account {
  return new Account(session.accountId, provider, new KeyPairSigner(KeyPair.fromString(session.secretKey as `ed25519:${string}`)));
}

/** Text of the first failure inside a transaction outcome, or null. */
function failureOf(outcome: any): string | null {
  const find = (status: any): string | null =>
    status && typeof status === 'object' && 'Failure' in status ? JSON.stringify(status.Failure) : null;
  const top = find(outcome?.status);
  if (top) return top;
  for (const r of outcome?.receipts_outcome ?? []) {
    const f = find(r?.outcome?.status);
    if (f) return f;
  }
  return null;
}

/** Pulls the human part out of a NEAR failure JSON, e.g. the contract's panic text. */
export function failureMessage(failureJson: string): string {
  const m = /Smart contract panicked: ([^"\\]*)/.exec(failureJson);
  return m?.[1] ?? failureJson.slice(0, 300);
}

export async function callWorld(
  session: Session,
  method: string,
  args: Record<string, unknown>,
  alreadyDone: AlreadyDone,
): Promise<CallResult> {
  const account = playerAccount(session);

  if (config.relayerUrl !== '') {
    try {
      const { signedDelegate } = await account.createSignedMetaTransaction({
        receiverId: config.worldContractId,
        actions: [actions.functionCall(method, args, GAS_100_TGAS, 0n)],
        blockHeightTtl: DELEGATE_TTL_BLOCKS,
      });
      const ok = await relay(toBase64(encodeSignedDelegate(signedDelegate)));
      return { txHash: ok.txHash, via: 'relayer', result: ok.result };
    } catch (e) {
      if (e instanceof RelayerRefused) {
        // The relayer said no (never retried directly), or the contract call failed.
        if (e.code !== 'call_failed') throw e;
        const done = await alreadyDone(e.txHash);
        if (done) return done;
        throw new CallFailed(failureMessage(e.message), e.txHash);
      }
      if (!(e instanceof RelayerUnavailable)) throw e;
      // It may have been sent anyway: only go direct if the contract shows it did not happen.
      const done = await alreadyDone(e.txHash);
      if (done) return done;
    }
  }

  try {
    return await callDirect(account, method, args);
  } catch (e) {
    // A relayed attempt still in flight can land first; then this direct one is refused.
    if (e instanceof CallFailed) {
      const done = await alreadyDone(e.txHash);
      if (done) return done;
    }
    throw e;
  }
}

async function callDirect(account: Account, method: string, args: Record<string, unknown>): Promise<CallResult> {
  // Not callFunctionRaw: it throws its own error on a contract panic, without the tx hash.
  const outcome = await account.signAndSendTransaction({
    receiverId: config.worldContractId,
    actions: [actions.functionCall(method, args, GAS_100_TGAS, 0n)],
    waitUntil: 'EXECUTED_OPTIMISTIC',
    throwOnFailure: false,
  });
  const txHash = outcome.transaction_outcome.id;
  const failure = failureOf(outcome);
  if (failure) throw new CallFailed(failureMessage(failure), txHash);
  let result: unknown = null;
  try {
    result = getTransactionLastResult(outcome);
  } catch {
    result = null;
  }
  return { txHash, via: 'direct', result };
}
