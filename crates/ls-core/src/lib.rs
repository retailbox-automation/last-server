//! Last Server game rules.
//!
//! One crate, compiled into the NEAR contract and (later) into the browser. Everything here
//! must be bit-for-bit deterministic: integer math only, no floats, no clocks, no hash-map
//! iteration, and the only randomness is the seeded PRNG defined below.
//!
//! A dive is `(seed, entry, moves) -> outcome`. The entry carries what the contract knows
//! before the dive (start depth, the player's items, the graves left by other players);
//! the outcome says what the contract must change afterwards.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[cfg(any(test, feature = "bot"))]
pub mod bot;

/// Crate version, reported by the contract's `core_version` view.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// FNV-1a 64 of this file, computed by build.rs. Changes whenever the rules change.
pub const RULES_HASH: &str = env!("LS_CORE_RULES_HASH");

pub const W: i32 = 32;
pub const H: i32 = 32;
const CELLS: usize = (W * H) as usize;

pub const WALL: u8 = 0;
pub const FLOOR: u8 = 1;
pub const STAIRS: u8 = 2;

pub const MONSTERS_PER_LEVEL: usize = 8;
const ITEMS_PER_LEVEL: u32 = 6;
const ROOM_ATTEMPTS: u32 = 14;
const BASE_HP: i32 = 20;
const REGEN_EVERY: u32 = 5;
const RESPAWN_EVERY: u32 = 25;
const MONSTER_MIN_DIST: i32 = 6;
const RESPAWN_MIN_DIST: i32 = 8;
/// Depth beyond which monsters stop getting stronger (keeps i32 math far from overflow).
const DEPTH_CAP: u32 = 1000;

pub type ItemId = u32;

pub const ITEM_GOLD: ItemId = 1;
pub const ITEM_BLADE: ItemId = 2;
pub const ITEM_PLATE: ItemId = 3;
pub const ITEM_GEM: ItemId = 4;
/// Every item kind the rules can produce. The contract exposes each as a NEP-245 token id.
pub const ITEM_IDS: [ItemId; 4] = [ITEM_GOLD, ITEM_BLADE, ITEM_PLATE, ITEM_GEM];

pub fn item_name(id: ItemId) -> Option<&'static str> {
    match id {
        ITEM_GOLD => Some("gold"),
        ITEM_BLADE => Some("blade"),
        ITEM_PLATE => Some("plate"),
        ITEM_GEM => Some("gem"),
        _ => None,
    }
}

pub fn item_id(name: &str) -> Option<ItemId> {
    ITEM_IDS.iter().copied().find(|&id| item_name(id) == Some(name))
}

pub const MOVE_UP: u8 = 0;
pub const MOVE_DOWN: u8 = 1;
pub const MOVE_LEFT: u8 = 2;
pub const MOVE_RIGHT: u8 = 3;
/// Any byte other than 0..=3 is a wait. It still counts as a move.
pub const MOVE_WAIT: u8 = 4;

fn move_delta(mv: u8) -> (i32, i32) {
    match mv {
        MOVE_UP => (0, -1),
        MOVE_DOWN => (0, 1),
        MOVE_LEFT => (-1, 0),
        MOVE_RIGHT => (1, 0),
        _ => (0, 0),
    }
}

// ---------------------------------------------------------------------------------------------
// PRNG
// ---------------------------------------------------------------------------------------------

/// splitmix64 step, used to derive well-mixed seeds.
pub fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// xorshift64* generator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut s = seed;
        let v = splitmix64(&mut s);
        Rng { state: if v == 0 { 0x9E37_79B9_7F4A_7C15 } else { v } }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish value in `0..n` (multiply-shift on the high 32 bits); 0 when `n == 0`.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        (((self.next_u64() >> 32) * u64::from(n)) >> 32) as u32
    }
}

// ---------------------------------------------------------------------------------------------
// Public data
// ---------------------------------------------------------------------------------------------

/// Items left at a depth by a player who died there.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Grave {
    pub depth: u32,
    pub x: u8,
    pub y: u8,
    pub items: Vec<(ItemId, u32)>,
}

/// What the contract knows before the dive.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiveEntry {
    pub start_depth: u32,
    /// Items the player carries into the dive (their token balances).
    pub inventory: Vec<(ItemId, u32)>,
    /// Graves currently on the server. `DiveOutcome::looted_graves` indexes into this list.
    pub graves: Vec<Grave>,
}

/// What the contract must apply after the dive.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiveOutcome {
    pub final_depth: u32,
    pub died: bool,
    /// Moves processed before the dive ended (moves after death are ignored).
    pub moves_applied: u32,
    /// New items found in the dungeon (floor and monster drops): minted to the player.
    pub picked: Vec<(ItemId, u32)>,
    /// Indexes into `DiveEntry::graves` the player emptied: moved to the player.
    pub looted_graves: Vec<u32>,
    /// On death: what the player carried, minus the death toll. Becomes a new grave.
    pub dropped_into_grave: Vec<(ItemId, u32)>,
    /// On death: the death toll (one gold in ten). Burned.
    pub burned: Vec<(ItemId, u32)>,
    /// Where the player died (the new grave's position), if they died.
    pub death_pos: Option<(u8, u8)>,
    pub hp_left: u32,
    pub kills: u32,
}

/// Adds `amount` of `id` to a small item list, keeping first-insertion order.
pub fn add_item(list: &mut Vec<(ItemId, u32)>, id: ItemId, amount: u32) {
    if amount == 0 {
        return;
    }
    if let Some(entry) = list.iter_mut().find(|e| e.0 == id) {
        entry.1 = entry.1.saturating_add(amount);
    } else {
        list.push((id, amount));
    }
}

fn count_item(list: &[(ItemId, u32)], id: ItemId) -> u32 {
    list.iter().filter(|e| e.0 == id).map(|e| e.1).fold(0u32, u32::saturating_add)
}

// ---------------------------------------------------------------------------------------------
// Level generation
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monster {
    pub x: i32,
    pub y: i32,
    pub hp: i32,
    pub alive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FloorItem {
    pub x: i32,
    pub y: i32,
    pub item: ItemId,
    pub amount: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraveSpot {
    pub x: i32,
    pub y: i32,
    /// Index into `DiveEntry::graves`.
    pub index: u32,
    pub looted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    pub depth: u32,
    pub tiles: [u8; CELLS],
    pub start: (i32, i32),
    pub stairs: (i32, i32),
    pub monsters: Vec<Monster>,
    pub items: Vec<FloorItem>,
    pub graves: Vec<GraveSpot>,
}

impl Level {
    pub fn tile(&self, x: i32, y: i32) -> u8 {
        tile_at(&self.tiles, x, y)
    }

    pub fn passable(&self, x: i32, y: i32) -> bool {
        self.tile(x, y) != WALL
    }

    pub fn monster_at(&self, x: i32, y: i32) -> Option<usize> {
        self.monsters.iter().position(|m| m.alive && m.x == x && m.y == y)
    }
}

fn tile_at(tiles: &[u8; CELLS], x: i32, y: i32) -> u8 {
    if x < 0 || y < 0 || x >= W || y >= H {
        WALL
    } else {
        tiles[(y * W + x) as usize]
    }
}

fn set_tile(tiles: &mut [u8; CELLS], x: i32, y: i32, t: u8) {
    if x >= 0 && y >= 0 && x < W && y < H {
        tiles[(y * W + x) as usize] = t;
    }
}

fn depth_factor(depth: u32) -> i32 {
    depth.min(DEPTH_CAP) as i32
}

fn monster_hp(depth: u32) -> i32 {
    3 + depth_factor(depth) / 2
}

/// Random passable tile; falls back to the first passable tile in scan order.
fn random_floor(tiles: &[u8; CELLS], rng: &mut Rng) -> (i32, i32) {
    for _ in 0..64 {
        let x = rng.below(W as u32) as i32;
        let y = rng.below(H as u32) as i32;
        if tile_at(tiles, x, y) == FLOOR {
            return (x, y);
        }
    }
    for i in 0..CELLS {
        if tiles[i] == FLOOR {
            return (i as i32 % W, i as i32 / W);
        }
    }
    (0, 0)
}

/// Random floor tile at least `min_dist` (Manhattan) from `from` with no live monster on it.
fn free_spot(
    tiles: &[u8; CELLS],
    monsters: &[Monster],
    rng: &mut Rng,
    from: (i32, i32),
    min_dist: i32,
) -> Option<(i32, i32)> {
    for _ in 0..64 {
        let x = rng.below(W as u32) as i32;
        let y = rng.below(H as u32) as i32;
        if tile_at(tiles, x, y) != FLOOR {
            continue;
        }
        if (x - from.0).abs() + (y - from.1).abs() < min_dist {
            continue;
        }
        if monsters.iter().any(|m| m.alive && m.x == x && m.y == y) {
            continue;
        }
        return Some((x, y));
    }
    None
}

/// The given cell if passable, otherwise the nearest passable cell (ring by ring, fixed order).
fn snap_to_floor(tiles: &[u8; CELLS], x: i32, y: i32) -> (i32, i32) {
    if tile_at(tiles, x, y) != WALL {
        return (x, y);
    }
    for r in 1..W.max(H) {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dy.abs() != r {
                    continue;
                }
                if tile_at(tiles, x + dx, y + dy) != WALL {
                    return (x + dx, y + dy);
                }
            }
        }
    }
    (x, y)
}

fn level_rng(seed: u64, depth: u32) -> Rng {
    Rng::new(seed ^ u64::from(depth).wrapping_mul(0xD1B5_4A32_D192_ED03))
}

/// Builds the map of `depth` for `seed`. Same `(seed, depth)` gives the same map for every
/// player, which is what lets a grave written by one dive be found by another.
pub fn generate_level(seed: u64, depth: u32, graves: &[Grave]) -> Level {
    let mut rng = level_rng(seed, depth);
    let mut tiles = [WALL; CELLS];
    // (x, y, w, h)
    let mut rooms: Vec<(i32, i32, i32, i32)> = Vec::new();

    for _ in 0..ROOM_ATTEMPTS {
        let w = 4 + rng.below(6) as i32;
        let h = 3 + rng.below(5) as i32;
        let x = 1 + rng.below((W - w - 1) as u32) as i32;
        let y = 1 + rng.below((H - h - 1) as u32) as i32;
        let overlaps = rooms.iter().any(|&(rx, ry, rw, rh)| {
            x < rx + rw + 1 && rx < x + w + 1 && y < ry + rh + 1 && ry < y + h + 1
        });
        if overlaps {
            continue;
        }
        for yy in y..y + h {
            for xx in x..x + w {
                set_tile(&mut tiles, xx, yy, FLOOR);
            }
        }
        if let Some(&(px, py, pw, ph)) = rooms.last() {
            let (ax, ay) = (px + pw / 2, py + ph / 2);
            let (bx, by) = (x + w / 2, y + h / 2);
            let horizontal_first = rng.below(2) == 0;
            let corner = if horizontal_first { (bx, ay) } else { (ax, by) };
            carve_line(&mut tiles, (ax, ay), corner);
            carve_line(&mut tiles, corner, (bx, by));
        }
        rooms.push((x, y, w, h));
    }

    // The first attempt can never overlap, so there is at least one room.
    let (fx, fy, fw, fh) = rooms[0];
    let start = (fx + fw / 2, fy + fh / 2);
    let stairs = if rooms.len() > 1 {
        let (lx, ly, lw, lh) = rooms[rooms.len() - 1];
        (lx + lw / 2, ly + lh / 2)
    } else {
        (fx + fw - 1, fy + fh - 1)
    };
    set_tile(&mut tiles, stairs.0, stairs.1, STAIRS);

    let mut grave_spots = Vec::new();
    for (i, g) in graves.iter().enumerate() {
        if g.depth == depth {
            let (gx, gy) = snap_to_floor(&tiles, i32::from(g.x), i32::from(g.y));
            grave_spots.push(GraveSpot { x: gx, y: gy, index: i as u32, looted: false });
        }
    }

    let mut monsters = Vec::with_capacity(MONSTERS_PER_LEVEL);
    for _ in 0..MONSTERS_PER_LEVEL {
        if let Some((mx, my)) = free_spot(&tiles, &monsters, &mut rng, start, MONSTER_MIN_DIST) {
            monsters.push(Monster { x: mx, y: my, hp: monster_hp(depth), alive: true });
        }
    }

    let mut items = Vec::new();
    let d = depth_factor(depth) as u32;
    for _ in 0..ITEMS_PER_LEVEL {
        let (ix, iy) = random_floor(&tiles, &mut rng);
        let roll = rng.below(100);
        let (item, amount) = if roll < 55 {
            (ITEM_GOLD, 1 + rng.below(5) + d / 2)
        } else if roll < 75 {
            (ITEM_BLADE, 1)
        } else if roll < 93 {
            (ITEM_PLATE, 1)
        } else {
            (ITEM_GEM, 1)
        };
        if (ix, iy) != start {
            items.push(FloorItem { x: ix, y: iy, item, amount });
        }
    }

    Level { depth, tiles, start, stairs, monsters, items, graves: grave_spots }
}

/// Carves an axis-aligned segment (one of the two coordinates must match).
fn carve_line(tiles: &mut [u8; CELLS], from: (i32, i32), to: (i32, i32)) {
    let (mut x, mut y) = from;
    let sx = (to.0 - x).signum();
    let sy = (to.1 - y).signum();
    loop {
        set_tile(tiles, x, y, FLOOR);
        if (x, y) == to {
            break;
        }
        x += sx;
        y += sy;
    }
}

// ---------------------------------------------------------------------------------------------
// The dive
// ---------------------------------------------------------------------------------------------

/// Step-by-step dive state. `run_dive` is the contract's entry point; the stepper is public so
/// the browser (and the test bot) can play one move at a time with exactly the same rules.
#[derive(Clone, Debug)]
pub struct Dive<'a> {
    seed: u64,
    entry: &'a DiveEntry,
    pub level: Level,
    pub depth: u32,
    pub x: i32,
    pub y: i32,
    pub hp: i32,
    carried: Vec<(ItemId, u32)>,
    picked: Vec<(ItemId, u32)>,
    looted: Vec<u32>,
    rng: Rng,
    pub turn: u32,
    died: bool,
    death_pos: Option<(u8, u8)>,
    moves_applied: u32,
    kills: u32,
}

impl<'a> Dive<'a> {
    pub fn new(seed: u64, entry: &'a DiveEntry) -> Self {
        let depth = entry.start_depth.max(1);
        let level = generate_level(seed, depth, &entry.graves);
        let mut carried = Vec::new();
        for &(id, amount) in &entry.inventory {
            add_item(&mut carried, id, amount);
        }
        let (x, y) = level.start;
        let mut dive = Dive {
            seed,
            entry,
            level,
            depth,
            x,
            y,
            hp: 0,
            carried,
            picked: Vec::new(),
            looted: Vec::new(),
            rng: Rng::new(seed ^ 0xA076_1D64_78BD_642F),
            turn: 0,
            died: false,
            death_pos: None,
            moves_applied: 0,
            kills: 0,
        };
        dive.hp = dive.max_hp();
        dive
    }

    pub fn is_dead(&self) -> bool {
        self.died
    }

    pub fn max_hp(&self) -> i32 {
        BASE_HP + 2 * count_item(&self.carried, ITEM_GEM).min(5) as i32
    }

    fn attack_bonus(&self) -> i32 {
        count_item(&self.carried, ITEM_BLADE).min(3) as i32
    }

    fn armor(&self) -> i32 {
        count_item(&self.carried, ITEM_PLATE).min(2) as i32
    }

    /// Applies one move. Returns false (and does nothing) once the player is dead.
    pub fn step(&mut self, mv: u8) -> bool {
        if self.died {
            return false;
        }
        self.moves_applied += 1;
        self.turn += 1;

        let (dx, dy) = move_delta(mv);
        if dx != 0 || dy != 0 {
            let (nx, ny) = (self.x + dx, self.y + dy);
            if let Some(i) = self.level.monster_at(nx, ny) {
                self.player_attack(i);
            } else if self.level.passable(nx, ny) {
                self.x = nx;
                self.y = ny;
                self.pick_up();
                if self.level.tile(nx, ny) == STAIRS {
                    self.descend();
                    self.end_of_turn();
                    return true;
                }
            }
            // Into a wall: no-op, the move still counts.
        }

        self.monsters_act();
        if !self.died {
            self.end_of_turn();
        }
        true
    }

    fn player_attack(&mut self, i: usize) {
        let dmg = 1 + self.rng.below(4) as i32 + self.attack_bonus();
        let m = &mut self.level.monsters[i];
        m.hp -= dmg;
        if m.hp <= 0 {
            m.alive = false;
            let (mx, my) = (m.x, m.y);
            self.kills += 1;
            if self.rng.below(2) == 0 {
                let amount = 1 + self.rng.below(3);
                self.level.items.push(FloorItem { x: mx, y: my, item: ITEM_GOLD, amount });
            }
        }
    }

    fn pick_up(&mut self) {
        let mut i = 0;
        while i < self.level.items.len() {
            let it = &self.level.items[i];
            if it.x == self.x && it.y == self.y {
                let it = self.level.items.remove(i);
                add_item(&mut self.carried, it.item, it.amount);
                add_item(&mut self.picked, it.item, it.amount);
            } else {
                i += 1;
            }
        }
        for gi in 0..self.level.graves.len() {
            let spot = &self.level.graves[gi];
            if spot.looted || spot.x != self.x || spot.y != self.y {
                continue;
            }
            let index = spot.index;
            self.level.graves[gi].looted = true;
            self.looted.push(index);
            for &(id, amount) in &self.entry.graves[index as usize].items {
                add_item(&mut self.carried, id, amount);
            }
        }
    }

    fn descend(&mut self) {
        self.depth += 1;
        self.level = generate_level(self.seed, self.depth, &self.entry.graves);
        self.x = self.level.start.0;
        self.y = self.level.start.1;
    }

    fn monsters_act(&mut self) {
        let d = depth_factor(self.depth);
        for i in 0..self.level.monsters.len() {
            let m = &self.level.monsters[i];
            if !m.alive {
                continue;
            }
            if (m.x - self.x).abs() + (m.y - self.y).abs() == 1 {
                if self.rng.below(100) < 60 {
                    let dmg = 1 + self.rng.below(2 + (d / 6) as u32) as i32 - self.armor();
                    if dmg > 0 {
                        self.hp -= dmg;
                        if self.hp <= 0 {
                            self.hp = 0;
                            self.died = true;
                            self.death_pos = Some((self.x as u8, self.y as u8));
                            return;
                        }
                    }
                }
            } else {
                self.monster_step(i);
            }
        }
    }

    /// Greedy step toward the player: larger axis first, then the other; blocked by walls,
    /// other monsters and the player.
    fn monster_step(&mut self, i: usize) {
        let (mx, my) = (self.level.monsters[i].x, self.level.monsters[i].y);
        let ddx = self.x - mx;
        let ddy = self.y - my;
        let along_x = (ddx.signum(), 0);
        let along_y = (0, ddy.signum());
        let order = if ddx.abs() >= ddy.abs() { [along_x, along_y] } else { [along_y, along_x] };
        for (cx, cy) in order {
            if cx == 0 && cy == 0 {
                continue;
            }
            let (tx, ty) = (mx + cx, my + cy);
            if (tx == self.x && ty == self.y)
                || !self.level.passable(tx, ty)
                || self.level.monster_at(tx, ty).is_some()
            {
                continue;
            }
            self.level.monsters[i].x = tx;
            self.level.monsters[i].y = ty;
            return;
        }
    }

    fn end_of_turn(&mut self) {
        if self.turn % REGEN_EVERY == 0 && self.hp < self.max_hp() {
            self.hp += 1;
        }
        if self.turn % RESPAWN_EVERY == 0 {
            self.respawn();
        }
    }

    fn respawn(&mut self) {
        let alive = self.level.monsters.iter().filter(|m| m.alive).count();
        if alive >= MONSTERS_PER_LEVEL {
            return;
        }
        let spot = free_spot(
            &self.level.tiles,
            &self.level.monsters,
            &mut self.rng,
            (self.x, self.y),
            RESPAWN_MIN_DIST,
        );
        if let Some((x, y)) = spot {
            let m = Monster { x, y, hp: monster_hp(self.depth), alive: true };
            if let Some(slot) = self.level.monsters.iter_mut().find(|m| !m.alive) {
                *slot = m;
            } else {
                self.level.monsters.push(m);
            }
        }
    }

    pub fn finish(self) -> DiveOutcome {
        let mut dropped = Vec::new();
        let mut burned = Vec::new();
        if self.died {
            for &(id, amount) in &self.carried {
                let toll = if id == ITEM_GOLD { amount / 10 } else { 0 };
                add_item(&mut burned, id, toll);
                add_item(&mut dropped, id, amount - toll);
            }
        }
        DiveOutcome {
            final_depth: self.depth,
            died: self.died,
            moves_applied: self.moves_applied,
            picked: self.picked,
            looted_graves: self.looted,
            dropped_into_grave: dropped,
            burned,
            death_pos: self.death_pos,
            hp_left: self.hp.max(0) as u32,
            kills: self.kills,
        }
    }
}

/// Re-runs a whole dive. This is what the contract executes on `settle_dive`.
pub fn run_dive(seed: u64, entry: &DiveEntry, moves: &[u8]) -> DiveOutcome {
    let mut dive = Dive::new(seed, entry);
    for &mv in moves {
        if !dive.step(mv) {
            break;
        }
    }
    dive.finish()
}

#[cfg(test)]
mod tests;
