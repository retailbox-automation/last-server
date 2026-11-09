import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { summarizeOutcome } from '../src/chain.ts';
import { WORLD } from './helpers.ts';

const b64 = (s: string) => Buffer.from(s).toString('base64');

describe('summarizeOutcome', () => {
  it('takes the overall SuccessValue', () => {
    const r = summarizeOutcome({ status: { SuccessValue: b64('{"a":1}') }, transaction_outcome: { id: 'H' } }, WORLD);
    assert.deepEqual(r, { txHash: 'H', ok: true, resultBase64: b64('{"a":1}') });
  });

  it('falls back to the world contract receipt when the overall value is empty', () => {
    const r = summarizeOutcome(
      {
        status: { SuccessValue: '' },
        transaction_outcome: { id: 'H' },
        receipts_outcome: [
          { outcome: { executor_id: 'bob.lastserver.testnet', status: { SuccessReceiptId: 'x' } } },
          { outcome: { executor_id: WORLD, status: { SuccessValue: b64('"ok"') } } },
        ],
      },
      WORLD,
    );
    assert.equal(r.ok, true);
    assert.equal(r.resultBase64, b64('"ok"'));
  });

  it('ignores a SuccessValue from some other contract', () => {
    const r = summarizeOutcome(
      {
        status: { SuccessValue: '' },
        transaction_outcome: { id: 'H' },
        receipts_outcome: [{ outcome: { executor_id: 'other.testnet', status: { SuccessValue: b64('"x"') } } }],
      },
      WORLD,
    );
    assert.deepEqual(r, { txHash: 'H', ok: true });
  });

  it('reports a top-level failure', () => {
    const r = summarizeOutcome(
      { status: { Failure: { ActionError: { kind: 'DelegateActionExpired' } } }, transaction_outcome: { id: 'H' } },
      WORLD,
    );
    assert.equal(r.ok, false);
    assert.match(r.failure ?? '', /DelegateActionExpired/);
  });

  it('reports a failed receipt even when the top-level status looks fine', () => {
    const r = summarizeOutcome(
      {
        status: { SuccessValue: '' },
        transaction_outcome: { id: 'H' },
        receipts_outcome: [
          { outcome: { executor_id: WORLD, status: { Failure: { ActionError: { kind: 'FunctionCallError' } } } } },
        ],
      },
      WORLD,
    );
    assert.equal(r.ok, false);
    assert.match(r.failure ?? '', /FunctionCallError/);
  });
});
