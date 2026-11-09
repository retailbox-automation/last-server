import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { actions, encodeSignedDelegate } from 'near-api-js';
import {
  DELEGATE_MAX_TTL_BLOCKS,
  DELEGATE_MIN_BLOCKS_LEFT,
  MAX_GAS,
  Rejection,
  checkDelegateWindow,
  playerLabel,
  playerNameProblem,
  validateSignedDelegate,
} from '../src/validate.ts';
import { PARENT, WORLD, delegateBase64, policy, signDelegate, throwawayKey } from './helpers.ts';

async function rejected(input: unknown): Promise<Rejection> {
  try {
    validateSignedDelegate(input, policy());
  } catch (e) {
    assert.ok(e instanceof Rejection, `expected Rejection, got ${String(e)}`);
    return e;
  }
  assert.fail('expected the delegate to be rejected');
}

describe('validateSignedDelegate: accepted', () => {
  it('decodes a real borsh SignedDelegate for start_dive', async () => {
    const key = throwawayKey();
    const signed = await signDelegate({ key, method: 'start_dive', args: { loadout: [['1', '2']] }, nonce: 42n });
    const b64 = Buffer.from(encodeSignedDelegate(signed)).toString('base64');
    const v = validateSignedDelegate(b64, policy());
    assert.equal(v.senderId, `bob.${PARENT}`);
    assert.equal(v.method, 'start_dive');
    assert.equal(v.gas, MAX_GAS);
    // The rebuilt object serializes to exactly the bytes that came in.
    assert.equal(Buffer.from(encodeSignedDelegate(v.signedDelegate)).toString('base64'), b64);
    assert.equal(v.signedDelegate.delegateAction.nonce, 42n);
    assert.equal(v.signedDelegate.delegateAction.publicKey.toString(), key.getPublicKey().toString());
  });

  it('accepts settle_dive with a 4000-move log (base64 inside JSON args)', async () => {
    const moves = Buffer.alloc(4000, 1).toString('base64');
    const v = validateSignedDelegate(await delegateBase64({ method: 'settle_dive', args: { dive_id: '1', moves } }), policy());
    assert.equal(v.method, 'settle_dive');
  });

  it('accepts gas below the ceiling', async () => {
    const v = validateSignedDelegate(await delegateBase64({ gas: 30_000_000_000_000n }), policy());
    assert.equal(v.gas, 30_000_000_000_000n);
  });
});

describe('checkDelegateWindow', () => {
  const code = (max: bigint, height: bigint): string | null => {
    try {
      checkDelegateWindow(max, height);
      return null;
    } catch (e) {
      assert.ok(e instanceof Rejection);
      assert.equal(e.status, 403);
      return e.code;
    }
  };

  it('passes the maxBlockHeight through from the signed bytes', async () => {
    const v = validateSignedDelegate(await delegateBase64({ maxBlockHeight: 123_456n }), policy());
    assert.equal(v.maxBlockHeight, 123_456n);
  });

  it('expired, about to expire, fine, too long', () => {
    const h = 1_000n;
    assert.equal(code(h - 1n, h), 'delegate_expired');
    assert.equal(code(h, h), 'delegate_expired');
    assert.equal(code(h + DELEGATE_MIN_BLOCKS_LEFT - 1n, h), 'delegate_expired');
    assert.equal(code(h + DELEGATE_MIN_BLOCKS_LEFT, h), null);
    assert.equal(code(h + 100n, h), null); // what the web signs
    assert.equal(code(h + DELEGATE_MAX_TTL_BLOCKS, h), null);
    assert.equal(code(h + DELEGATE_MAX_TTL_BLOCKS + 1n, h), 'delegate_ttl_too_long');
    assert.equal(code(2n ** 64n - 1n, h), 'delegate_ttl_too_long');
  });
});

describe('validateSignedDelegate: 403 policy rejections', () => {
  it('wrong receiver', async () => {
    const r = await rejected(await delegateBase64({ receiver: 'evil.testnet' }));
    assert.equal(r.status, 403);
    assert.equal(r.code, 'wrong_receiver');
  });

  it('studio account as receiver is not the world contract', async () => {
    const r = await rejected(await delegateBase64({ receiver: PARENT }));
    assert.equal(r.code, 'wrong_receiver');
  });

  it('wrong method', async () => {
    for (const method of ['mt_transfer', 'new', 'expire_dive', 'start_dive2', '']) {
      const r = await rejected(await delegateBase64({ method }));
      assert.equal(r.status, 403, method);
      assert.equal(r.code, 'forbidden_method', method);
    }
  });

  it('deposit above zero', async () => {
    for (const deposit of [1n, 10n ** 24n]) {
      const r = await rejected(await delegateBase64({ method: 'mt_transfer', deposit }));
      assert.equal(r.status, 403);
      // method check runs first, so use an allowed method for the pure deposit case:
      const r2 = await rejected(await delegateBase64({ deposit }));
      assert.equal(r2.code, 'deposit_not_allowed');
    }
  });

  it('gas above 100 Tgas', async () => {
    const r = await rejected(await delegateBase64({ gas: MAX_GAS + 1n }));
    assert.equal(r.status, 403);
    assert.equal(r.code, 'gas_too_high');
    const r2 = await rejected(await delegateBase64({ gas: 300_000_000_000_000n }));
    assert.equal(r2.code, 'gas_too_high');
  });

  it('zero gas', async () => {
    assert.equal((await rejected(await delegateBase64({ gas: 0n }))).code, 'gas_zero');
  });

  it('sender is not a direct sub-account of the studio account', async () => {
    const senders = [
      'bob.testnet',
      PARENT,
      `a.b.${PARENT}`,
      `bob.${PARENT}.evil.testnet`,
      `world.${PARENT}`, // reserved label
      `Bob.${PARENT}`,
      `ab.${PARENT}`, // too short
      `a--b.${PARENT}`,
    ];
    for (const sender of senders) {
      const r = await rejected(await delegateBase64({ sender }));
      assert.equal(r.status, 403, sender);
      assert.equal(r.code, 'wrong_sender', sender);
    }
  });

  it('non-FunctionCall action', async () => {
    const r = await rejected(await delegateBase64({ actions: [actions.transfer(1n)] }));
    assert.equal(r.status, 403);
    assert.equal(r.code, 'forbidden_action');
    const r2 = await rejected(await delegateBase64({ actions: [actions.addFullAccessKey(throwawayKey().getPublicKey())] }));
    assert.equal(r2.code, 'forbidden_action');
  });

  it('more than one action, or none', async () => {
    const call = () => actions.functionCall('start_dive', {}, 1_000_000n, 0n);
    assert.equal((await rejected(await delegateBase64({ actions: [call(), call()] }))).code, 'bad_actions');
    assert.equal((await rejected(await delegateBase64({ actions: [call(), actions.transfer(5n)] }))).code, 'bad_actions');
    assert.equal((await rejected(await delegateBase64({ actions: [] }))).code, 'bad_actions');
  });

  it('call arguments over the size cap', async () => {
    const r = await rejected(await delegateBase64({ args: new Uint8Array(9000) }));
    assert.equal(r.status, 403);
    assert.equal(r.code, 'args_too_large');
  });

  it('signature made by another key', async () => {
    const r = await rejected(await delegateBase64({ key: throwawayKey(), publicKeyOf: throwawayKey() }));
    assert.equal(r.status, 403);
    assert.equal(r.code, 'bad_signature');
  });

  it('a signed delegate whose bytes were altered after signing', async () => {
    const signed = await signDelegate({ nonce: 7n });
    const bytes = Buffer.from(encodeSignedDelegate(signed));
    // Flip the last byte of the signature.
    bytes[bytes.length - 1] = bytes[bytes.length - 1]! ^ 0xff;
    const r = await rejected(bytes.toString('base64'));
    assert.equal(r.status, 403);
    assert.equal(r.code, 'bad_signature');
  });

  it('a different nonce under the same signature', async () => {
    const signed = await signDelegate({ nonce: 7n });
    signed.delegateAction.nonce = 8n;
    const r = await rejected(Buffer.from(encodeSignedDelegate(signed)).toString('base64'));
    assert.equal(r.code, 'bad_signature');
  });
});

describe('validateSignedDelegate: 400 undecodable input', () => {
  it('not a string', async () => {
    for (const input of [undefined, null, 5, {}, ['a']]) {
      const r = await rejected(input);
      assert.equal(r.status, 400);
      assert.equal(r.code, 'bad_delegate');
    }
  });

  it('not base64', async () => {
    for (const input of ['', '***', 'abc', 'not base64 at all!!', 'AAAA\nAAAA']) {
      const r = await rejected(input);
      assert.equal(r.status, 400, JSON.stringify(input));
    }
  });

  it('valid base64 but not a SignedDelegate', async () => {
    const r = await rejected(Buffer.from('hello world, this is not borsh').toString('base64'));
    assert.equal(r.status, 400);
    assert.equal(r.code, 'bad_delegate');
  });

  it('truncated', async () => {
    const bytes = Buffer.from(encodeSignedDelegate(await signDelegate()));
    const r = await rejected(bytes.subarray(0, bytes.length - 10).toString('base64'));
    assert.equal(r.status, 400);
  });

  it('trailing bytes after a valid SignedDelegate', async () => {
    const bytes = Buffer.concat([Buffer.from(encodeSignedDelegate(await signDelegate())), Buffer.from([1, 2, 3])]);
    const r = await rejected(bytes.toString('base64'));
    assert.equal(r.status, 400);
  });

  it('far too large', async () => {
    const r = await rejected(Buffer.alloc(40_000, 1).toString('base64'));
    assert.equal(r.status, 400);
    assert.equal(r.code, 'delegate_too_large');
  });
});

describe('player names', () => {
  it('accepts the spec pattern', () => {
    for (const n of ['abc', 'tom', 'mara-7', 'a1b', 'x'.repeat(24), '007']) {
      assert.equal(playerNameProblem(n), null, n);
    }
  });

  it('rejects everything else', () => {
    const bad = [
      '',
      'ab',
      'x'.repeat(25),
      '-abc',
      'abc-',
      'Abc',
      'a_b',
      'a.b',
      'a b',
      'a--b',
      'tom@example.com',
      'world',
      'world-demo',
      'lastserver',
      'admin',
      42,
      null,
      undefined,
    ];
    for (const n of bad) assert.notEqual(playerNameProblem(n), null, String(n));
  });

  it('playerLabel only accepts direct sub-accounts', () => {
    assert.equal(playerLabel(`tom.${PARENT}`, PARENT), 'tom');
    assert.equal(playerLabel(`a.b.${PARENT}`, PARENT), null);
    assert.equal(playerLabel(WORLD, PARENT), null);
    assert.equal(playerLabel('tom.other.testnet', PARENT), null);
  });
});
