// NearChain against a local JSON-RPC stub that answers with the exact bodies test.rpc.fastnear.com
// returned on 2026-11-09 (read-only `query` probes). No network.

import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import type { Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { createHash } from 'node:crypto';
import { after, before, describe, it } from 'node:test';
import { baseEncode, decodeSignedTransaction, encodeTransaction } from 'near-api-js';
import { ChainError, NearChain } from '../src/chain.ts';
import { loadConfig } from '../src/config.ts';
import { PARENT, WORLD, signDelegate, throwawayKey } from './helpers.ts';

const BLOCK = { block_hash: '33Q2TsM8apFrQtKgtQq7srqv4g7FJTEtimEuVa3ZiW35', block_height: 276936993 };

/** Accounts that exist on the stub chain: account id -> its one full-access key. */
const accounts = new Map<string, string>();
const seen: Array<{ request_type: string; finality?: string }> = [];
const studioKey = throwawayKey();
accounts.set(PARENT, studioKey.getPublicKey().toString());
/** What send_tx answers next, and the hash of the transaction it received. */
let sendReply: 'timeout' | 'invalid_tx' = 'timeout';
let sentHash = '';

function answer(method: string, params: Record<string, string>): unknown {
  if (method === 'block') {
    return { result: { header: { height: BLOCK.block_height, hash: BLOCK.block_hash }, chunks: [], author: 'x' } };
  }
  if (method === 'send_tx') {
    const signed = decodeSignedTransaction(Buffer.from(params.signed_tx_base64!, 'base64'));
    sentHash = baseEncode(createHash('sha256').update(encodeTransaction(signed.transaction)).digest());
    const cause = sendReply === 'timeout' ? { name: 'TIMEOUT_ERROR', info: {} } : { name: 'INVALID_TRANSACTION', info: {} };
    return { error: { name: 'HANDLER_ERROR', cause, code: -32000, message: 'Server error', data: cause.name } };
  }
  seen.push({ request_type: params.request_type!, finality: params.finality });
  const id = params.account_id!;
  if (params.request_type === 'view_access_key') {
    // Live testnet: a missing ACCOUNT is reported as a missing access key, in `result.error`.
    if (accounts.get(id) !== params.public_key) {
      return { result: { ...BLOCK, error: `access key ${params.public_key} does not exist while viewing`, logs: [] } };
    }
    return { result: { ...BLOCK, nonce: 5, permission: 'FullAccess' } };
  }
  if (params.request_type === 'view_account') {
    if (!accounts.has(id)) {
      return {
        error: {
          name: 'HANDLER_ERROR',
          cause: { info: { ...BLOCK, requested_account_id: id }, name: 'UNKNOWN_ACCOUNT' },
          code: -32000,
          message: 'Server error',
          data: `account ${id} does not exist while viewing`,
        },
      };
    }
    return {
      result: {
        ...BLOCK,
        amount: '2000000000000000000000000',
        locked: '0',
        code_hash: '11111111111111111111111111111111',
        storage_usage: 182,
        storage_paid_at: 0,
      },
    };
  }
  throw new Error(`stub: unexpected ${params.request_type}`);
}

let server: Server;
let chain: NearChain;

before(async () => {
  server = createServer((req, res) => {
    const chunks: Buffer[] = [];
    req.on('data', (c: Buffer) => chunks.push(c));
    req.on('end', () => {
      const body = JSON.parse(Buffer.concat(chunks).toString());
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ jsonrpc: '2.0', id: body.id, ...(answer(body.method, body.params) as object) }));
    });
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  const rpcUrl = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  chain = new NearChain(
    loadConfig({
      LASTSERVER_ACCOUNT_ID: PARENT,
      LASTSERVER_SECRET_KEY: studioKey.toString(),
      WORLD_CONTRACT_ID: WORLD,
      RPC_URL: rpcUrl,
    }),
  );
});
after(() => server.close());

describe('NearChain.accountStatus (testnet RPC answers)', () => {
  it('a name nobody holds is "absent" (live RPC reports it as a missing access key)', async () => {
    const pk = throwawayKey().getPublicKey().toString();
    assert.equal(await chain.accountStatus(`nobody-x7q2k9.${PARENT}`, pk), 'absent');
  });

  it('the same key is "has_key", another key is "other_key"', async () => {
    const mine = throwawayKey().getPublicKey().toString();
    accounts.set(`mara.${PARENT}`, mine);
    assert.equal(await chain.accountStatus(`mara.${PARENT}`, mine), 'has_key');
    assert.equal(await chain.accountStatus(`mara.${PARENT}`, throwawayKey().getPublicKey().toString()), 'other_key');
  });

  it('isAccessKey is true only for the key the account holds; another key and a missing account are false', async () => {
    const mine = throwawayKey().getPublicKey().toString();
    accounts.set(`rhea.${PARENT}`, mine);
    assert.equal(await chain.isAccessKey(`rhea.${PARENT}`, mine), true);
    assert.equal(await chain.isAccessKey(`rhea.${PARENT}`, throwawayKey().getPublicKey().toString()), false);
    assert.equal(await chain.isAccessKey(`nobody-y4m1.${PARENT}`, mine), false);
  });

  it('reads at optimistic finality, so an account created a moment ago is already visible', async () => {
    seen.length = 0;
    await chain.accountStatus(`nobody-2.${PARENT}`, throwawayKey().getPublicKey().toString());
    assert.ok(seen.length > 0);
    for (const q of seen) assert.equal(q.finality, 'optimistic', q.request_type);
  });
});

describe('NearChain.relay: sent vs not sent', () => {
  it('a timeout after the transaction was sent is relay_unknown and names the sent transaction', async () => {
    sendReply = 'timeout';
    const signed = await signDelegate({ sender: `mara.${PARENT}` });
    await assert.rejects(chain.relay(signed, WORLD), (e: unknown) => {
      assert.ok(e instanceof ChainError);
      assert.equal(e.code, 'relay_unknown');
      assert.equal(e.txHash, sentHash);
      assert.ok(sentHash.length > 40);
      return true;
    });
  });

  it('a transaction the chain refused outright is chain_error (nothing executed)', async () => {
    sendReply = 'invalid_tx';
    const signed = await signDelegate({ sender: `mara.${PARENT}` });
    await assert.rejects(chain.relay(signed, WORLD), (e: unknown) => e instanceof ChainError && e.code === 'chain_error');
  });

  it('block height comes from the latest block', async () => {
    assert.equal(await chain.blockHeight(), BigInt(BLOCK.block_height));
  });
});
