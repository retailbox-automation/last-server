// Player keys live in this browser's localStorage: one ed25519 key per name. Anyone who can read the
// page's storage (an XSS bug, a shared browser profile) can play as that player, so the account only
// ever holds testnet NEAR and the page loads no third-party scripts.

import { KeyPair } from 'near-api-js';

const KEYS = 'lastserver.keys.v1'; // JSON { [name]: "ed25519:<secret>" }
const LAST = 'lastserver.last.v1'; // last name used

export interface Session {
  name: string;
  accountId: string;
  secretKey: string;
}

const memory = new Map<string, string>();

function read(k: string): string | null {
  try {
    return localStorage.getItem(k) ?? memory.get(k) ?? null;
  } catch {
    return memory.get(k) ?? null;
  }
}

function write(k: string, v: string): void {
  memory.set(k, v);
  try {
    localStorage.setItem(k, v);
  } catch {
    // Private window or blocked storage: the key then lives only until the tab closes.
  }
}

function readKeys(): Record<string, string> {
  try {
    const parsed: unknown = JSON.parse(read(KEYS) ?? '{}');
    return parsed !== null && typeof parsed === 'object' ? (parsed as Record<string, string>) : {};
  } catch {
    return {};
  }
}

export function newKeyPair(): KeyPair {
  return KeyPair.fromRandom('ed25519');
}

export function storedSecret(name: string): string | null {
  return readKeys()[name] ?? null;
}

export function saveSession(s: Session): void {
  const keys = readKeys();
  keys[s.name] = s.secretKey;
  write(KEYS, JSON.stringify(keys));
  write(LAST, s.name);
}

export function lastName(): string {
  return read(LAST) ?? '';
}

export function forgetLast(): void {
  write(LAST, '');
}
