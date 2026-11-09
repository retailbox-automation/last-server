// waitForAccessKey: a just-created account's key can be unknown to the RPC for a few seconds.
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { KeyPair, PublicKey } from 'near-api-js';
import { AccessKeyDoesNotExistError, AccountDoesNotExistError } from 'near-api-js/rpc-errors';
import { isMissingKey, waitForAccessKey } from '../src/chain.ts';

const pk = KeyPair.fromRandom('ed25519').getPublicKey();
const ACCOUNT = 'newbie.lastserver.testnet';

/** Fake RPC: the key is missing for the first `missing[finality]` lookups at each finality. */
function fakeRpc(missing: { optimistic: number; final: number }) {
  const calls: string[] = [];
  const left = { ...missing };
  return {
    calls,
    provider: {
      async viewAccessKey({ finalityQuery }: { accountId: string; publicKey: PublicKey; finalityQuery?: { finality: string } }) {
        const f = (finalityQuery?.finality ?? 'optimistic') as 'optimistic' | 'final';
        calls.push(f);
        if (left[f] > 0) {
          left[f]--;
          throw new AccessKeyDoesNotExistError(pk, 'HASH', 1);
        }
        return { nonce: 1n, permission: 'FullAccess', block_height: 1, block_hash: 'HASH' };
      },
    } as any,
  };
}

/** A clock that only moves when the code sleeps. */
function fakeClock() {
  let t = 0;
  const slept: number[] = [];
  return { now: () => t, sleep: async (ms: number) => { slept.push(ms); t += ms; }, slept };
}

describe('waitForAccessKey', () => {
  it('polls until the key shows at optimistic, then at final', async () => {
    const rpc = fakeRpc({ optimistic: 3, final: 2 });
    const clock = fakeClock();
    await waitForAccessKey(ACCOUNT, pk.toString(), { provider: rpc.provider, ...clock, intervalMs: 500 });
    assert.deepEqual(rpc.calls, ['optimistic', 'optimistic', 'optimistic', 'optimistic', 'final', 'final', 'final']);
    assert.deepEqual(clock.slept, [500, 500, 500, 500, 500]);
  });

  it('returns at once when the key is already there', async () => {
    const rpc = fakeRpc({ optimistic: 0, final: 0 });
    const clock = fakeClock();
    await waitForAccessKey(ACCOUNT, pk.toString(), { provider: rpc.provider, ...clock });
    assert.deepEqual(rpc.calls, ['optimistic', 'final']);
    assert.equal(clock.slept.length, 0);
  });

  it('gives up after the timeout with a plain message', async () => {
    const rpc = fakeRpc({ optimistic: 1_000, final: 0 });
    const clock = fakeClock();
    await assert.rejects(
      waitForAccessKey(ACCOUNT, pk.toString(), { provider: rpc.provider, ...clock, timeoutMs: 15_000, intervalMs: 1_000 }),
      (e: unknown) => e instanceof Error && /does not show its key yet/.test(e.message) && /press Play again/.test(e.message),
    );
    assert.equal(clock.now(), 15_000);
    assert.equal(rpc.calls.length, 16);
  });

  it('recognizes a missing key or account, nothing else', () => {
    assert.equal(isMissingKey(new AccessKeyDoesNotExistError(pk, 'H', 1)), true);
    assert.equal(isMissingKey(new AccountDoesNotExistError(ACCOUNT, 'H', 1)), true);
    assert.equal(isMissingKey(new Error('Access key does not exist')), false);
  });
});
