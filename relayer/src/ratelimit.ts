// Fixed-window counters keyed by string. Memory is bounded: expired windows are swept
// whenever the map grows past SWEEP_AT entries.

const SWEEP_AT = 10_000;

export interface HitResult {
  ok: boolean;
  /** Seconds until the window resets (only meaningful when ok is false). */
  retryAfterS: number;
}

export class RateLimiter {
  private readonly max: number;
  private readonly windowMs: number;
  private readonly now: () => number;
  private readonly windows = new Map<string, { start: number; count: number }>();

  constructor(max: number, windowS: number, now: () => number = Date.now) {
    this.max = max;
    this.windowMs = windowS * 1000;
    this.now = now;
  }

  hit(key: string): HitResult {
    const t = this.now();
    if (this.windows.size >= SWEEP_AT) this.sweep(t);
    let w = this.windows.get(key);
    if (!w || t - w.start >= this.windowMs) {
      w = { start: t, count: 0 };
      this.windows.set(key, w);
    }
    if (w.count >= this.max) {
      return { ok: false, retryAfterS: Math.max(1, Math.ceil((w.start + this.windowMs - t) / 1000)) };
    }
    w.count += 1;
    return { ok: true, retryAfterS: 0 };
  }

  private sweep(t: number): void {
    for (const [k, w] of this.windows) {
      if (t - w.start >= this.windowMs) this.windows.delete(k);
    }
  }
}
