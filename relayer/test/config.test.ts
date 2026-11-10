import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { loadConfig } from '../src/config.ts';
import { throwawayKey } from './helpers.ts';

const base = {
  LASTSERVER_ACCOUNT_ID: 'studio.testnet',
  // Fresh random key per run, never used on any network.
  LASTSERVER_SECRET_KEY: throwawayKey().toString(),
  WORLD_CONTRACT_ID: 'world.studio.testnet',
};

describe('config limits', () => {
  it('rejects a 0 s window: it would switch the limiter off', () => {
    assert.throws(() => loadConfig({ ...base, RATE_RELAY_GLOBAL_WINDOW_S: '0' }), /RATE_RELAY_GLOBAL_WINDOW_S must be an integer of at least 1/);
    assert.throws(() => loadConfig({ ...base, RATE_SIGNUP_IP_WINDOW_S: '0' }), /RATE_SIGNUP_IP_WINDOW_S/);
  });
  it('takes plain decimal integers only', () => {
    assert.throws(() => loadConfig({ ...base, RATE_RELAY_GLOBAL_MAX: '1e3' }), /RATE_RELAY_GLOBAL_MAX/);
    assert.throws(() => loadConfig({ ...base, RATE_RELAY_GLOBAL_MAX: '0x10' }), /RATE_RELAY_GLOBAL_MAX/);
    assert.equal(loadConfig({ ...base, RATE_RELAY_GLOBAL_MAX: '250' }).limits.relayGlobalMax, 250);
  });
});
