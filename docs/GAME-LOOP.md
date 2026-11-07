# Last Server — game loop on paper (before code)

Written 2026-11-07, before the real rules crate. Asked for by the organizers' games prompt (https://github.com/nearai/near-hackathon-prompt-examples/blob/main/tracks/09-games.md): "Design the game loop on paper first: what the player does every 30 seconds, every 5 minutes and every session". Track line: "Judged on the game loop, not the token — bonus if players never notice the chain." This is a draft for the build; numbers are starting values to tune with the fast-forward bots, not promises.

## The one sentence
Every dive is a bet: what you carry down can make you strong enough to go deeper, and everything you carry stays where you die, for the next player to take.

## Every 30 seconds — the room
- Step through a small tile dungeon (32×32 per depth, generated from the day's seed). Turn-based: one move = one input; monsters move after you.
- Bump to attack. Damage is rolled by the seeded RNG; HP does not regenerate during a dive (potions do).
- Pick up what is on the floor. Weapons wear out during a dive: each hit costs durability, and a weapon that breaks is burned when the dive settles (the economy's sink). Durability lives only inside a dive, so one item type stays one NEP-245 token id.
- See a grave marker on the map edge ("Mara · depth 6 · Blue Key"). Decide: detour to the grave (often in a monster-heavy spot) or keep going.

## Every 5 minutes — the dive
- Before the dive pick a loadout from your stash: what you carry is what you risk.
- Go down the stairs or turn back. Deeper = better drops and the graves of players who were better geared than you.
- Locked vaults: a coloured door at depth N needs the key of that colour, and keys drop deep. That is why Mara's Blue Key matters to Tom: it opens the depth-7 vault.
- Three endings: (1) **return to the surface** — everything you carry is banked to your stash; (2) **die** — everything you carry becomes your grave at that depth, with your name, for anyone to loot (including you, if you get there first: a corpse run); (3) **quit mid-dive** — counts as death (no free exit).
- When the dive ends the client sends the seed and your move log. The contract re-runs the same rules and decides what happened: "dive recorded". The client cannot invent loot, because the contract only accepts what the rules produce.

## Every session (a day)
- A new seed every day: the same dungeon for everyone that day, so stories are comparable ("I died at the depth-6 bend too").
- Graves stay across days (they belong to a depth, not to a map), so the world fills with other people's stories.
- The surface map shows every grave: who, how deep, what they carried. This is the world's memory, and it lives on the chain.
- The demo moment: the studio stops its backend. The page still reads the world straight from the chain and dives still settle (players who already have a key keep playing; newcomers need a community relayer or their own gas — we say this plainly).

## The economy, without a dev patch
- **Faucet:** items drop in the dungeon (deeper = rarer).
- **Sink:** weapon wear and breakage (burn), potions used up.
- **Recirculation:** graves move items from the dead to the living; the deeper and more dangerous the depth, the richer its graves get, so risk is priced by the players themselves.
- **Fast-forward:** 500 scripted bots dive for a simulated week in about a minute with the same rules crate (in the browser, from a Fast-forward button, and as a CLI); the chart shows supply per item, how much was burnt, and how much went through graves. Judges can watch the economy move (prompt requirement).
- **Rules in one readable file:** all numbers (drop tables, damage, durability, depth curve) sit in one Rust file of the rules crate; the client shows "why" for a settled dive (which room, which roll, which grave).

## What we will not build (scope guard)
- No trading UI, no prices, no token for money. Items have use, not a price.
- No mainnet, no real money, no wallet popups.
- NPC agents with wallets (track hint) are a stretch, not the core; only if the core is done by Tue 10.11.

## Consequences for the contract (beyond the day-1 spike)
- Two calls per dive, not one: `start_dive(loadout)` moves the chosen items from the stash to "carried" (so the risk is real), then `settle_dive(seed, moves)` banks them or turns them into a grave. A dive not settled by the end of its day can be expired by anyone (`expire_dive`) → grave. Without this, closing the tab would be a free exit.
- The day's seed is public and shared (daily-run genre: comparable stories). Players can pre-plan a route on a known seed; acceptable for this game and stated in README. Revisit only if the bots show an exploit that breaks the economy.
- Graves are keyed by depth and placed on the day's map by a seeded position, so they persist across daily maps.

## Open questions to settle by numbers (fast-forward bots)
1. How many moves does a typical 5-minute dive take (target ≈300; checked against the gas spike)?
2. Grave density per depth after a simulated week: too many graves = free loot, too few = nobody finds anything.
3. Breakage rate that keeps supply flat over a week without making the first dive miserable.
