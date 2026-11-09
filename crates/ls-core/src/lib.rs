//! Last Server game rules.
//!
//! One crate, compiled into the NEAR contract and into the browser. Everything here must be
//! bit-for-bit deterministic: integer math only, no floats, no clocks, no hash-map iteration,
//! and the only randomness is the seeded PRNG defined below. Every tunable number lives in
//! `rules.rs`; this file is the engine that applies them.
//!
//! A dive is `(seed, entry, moves) -> outcome`. The browser plays it one move at a time with
//! [`DiveSim`]; the contract re-runs the whole log with [`run_dive`] / [`run_dive_with`], which
//! are the same `DiveSim::step` loop. Graves are read lazily, one depth at a time, when the dive
//! enters that depth ([`GraveSource`]), so the cost of a dive does not grow with the number of
//! graves on the server.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

pub mod rules;
pub use rules::{H, MAX_MOVES, START_DEPTH, W};

#[cfg(any(test, feature = "bot"))]
pub mod bot;

/// Crate version, reported by the contract's `core_version` view.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// sha256 of `rules.rs` (all the numbers), computed by build.rs.
pub const RULES_HASH: &str = env!("LS_CORE_RULES_HASH");
/// sha256 of `lib.rs` followed by `rules.rs` (numbers and engine), computed by build.rs.
pub const CORE_HASH: &str = env!("LS_CORE_CORE_HASH");

const CELLS: usize = (W * H) as usize;

pub const WALL: u8 = 0;
pub const FLOOR: u8 = 1;
pub const STAIRS: u8 = 2;
/// The vault door: a wall until the player walks into it carrying a blue key (the key is used
/// up and the door becomes floor for the rest of the dive). Monsters never pass it.
pub const DOOR_BLUE: u8 = 3;

pub type ItemId = u32;

pub const ITEM_GOLD: ItemId = 1;
pub const ITEM_BLADE: ItemId = 2;
pub const ITEM_PLATE: ItemId = 3;
pub const ITEM_GEM: ItemId = 4;
pub const ITEM_POTION: ItemId = 5;
pub const ITEM_BLUE_KEY: ItemId = 6;
/// Every item kind the rules can produce. The contract exposes each as a NEP-245 token id.
pub const ITEM_IDS: [ItemId; 6] = [ITEM_GOLD, ITEM_BLADE, ITEM_PLATE, ITEM_GEM, ITEM_POTION, ITEM_BLUE_KEY];

pub fn item_name(id: ItemId) -> Option<&'static str> {
    match id {
        ITEM_GOLD => Some("gold"),
        ITEM_BLADE => Some("blade"),
        ITEM_PLATE => Some("plate"),
        ITEM_GEM => Some("gem"),
        ITEM_POTION => Some("potion"),
        ITEM_BLUE_KEY => Some("blue_key"),
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
/// Wait one turn. Any byte without a meaning of its own is a wait; it still counts as a move.
pub const MOVE_WAIT: u8 = 4;
/// Drink a potion (heals `rules::POTION_HEAL`). Without a potion, or at full HP, it is a wait.
pub const MOVE_POTION: u8 = 5;
/// Return to the surface. Only on the tile where the dive arrived at its current depth (the way
/// up, `DiveView::exit_x` / `exit_y`); anywhere else it is a wait. Only a dive that returns banks
/// what it carries: a log that ends inside the dungeon counts as a death where the player stands.
pub const MOVE_SURFACE: u8 = 6;

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

/// Items left at a depth by a player who died there. `id` is assigned by the contract.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Grave {
    pub id: u32,
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
    /// Items the player carries into the dive (the locked loadout).
    pub inventory: Vec<(ItemId, u32)>,
    /// Graves the dive may meet, of any depths, in the order the contract stores them per depth.
    /// `DiveSim::new` reads the ones of a depth when the dive enters it. The contract leaves this
    /// empty and feeds graves through a [`GraveSource`] instead.
    pub graves: Vec<Grave>,
}

/// Graves of one depth, read once when a dive enters that depth.
pub trait GraveSource {
    /// The graves currently at `depth`, in a fixed order (the order matters to the outcome).
    fn graves_at(&mut self, depth: u32) -> Vec<Grave>;
}

/// A preloaded list is a source: it yields the graves of the requested depth, in list order.
impl GraveSource for &[Grave] {
    fn graves_at(&mut self, depth: u32) -> Vec<Grave> {
        self.iter().filter(|g| g.depth == depth).cloned().collect()
    }
}

/// What the contract must apply after the dive.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiveOutcome {
    pub final_depth: u32,
    /// The dive ends as a death: killed, or the log ended inside the dungeon (`quit`).
    pub died: bool,
    /// The log ends with a return to the surface (`MOVE_SURFACE` on the way up): what the player
    /// carries is banked.
    pub returned: bool,
    /// The log ended inside the dungeon, without a return (or it ran out of moves there). This
    /// counts as a death where the player stood: grave and death toll as for any death.
    pub quit: bool,
    /// Moves processed before the dive ended (moves after death or past `MAX_MOVES` are ignored).
    pub moves_applied: u32,
    /// New items found in the dungeon (floor and monster drops): minted to the player.
    pub picked: Vec<(ItemId, u32)>,
    /// Ids of the graves the player emptied: their items move to the player.
    pub looted_graves: Vec<u32>,
    /// On death: what the player carried at the end, minus the death toll. Becomes a new grave.
    pub dropped_into_grave: Vec<(ItemId, u32)>,
    /// Burned from the player: items used up or broken during the dive and, on death, the toll.
    pub burned: Vec<(ItemId, u32)>,
    /// Where the player died (the new grave's position), if they died.
    pub death_pos: Option<(u8, u8)>,
    pub hp_left: u32,
    pub kills: u32,
    pub blades_broken: u32,
    pub potions_used: u32,
    pub keys_used: u32,
    pub vault_opened: bool,
}

/// One thing that happened during a step, for the browser to animate and explain.
/// `monster` is the monster's index in `DiveView::monsters[].id` on the current depth.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
pub enum Event {
    /// The player walked to (x, y).
    Moved { x: i32, y: i32 },
    /// The player walked into a wall at (x, y); nothing moved.
    Bumped { x: i32, y: i32 },
    /// The player hit a monster.
    Attacked { monster: u32, damage: u32, killed: bool },
    /// A killed monster dropped an item on its tile.
    Dropped { x: i32, y: i32, item: ItemId, amount: u32 },
    /// A monster hit the player.
    Hurt { monster: u32, damage: u32 },
    /// An adjacent monster missed.
    Missed { monster: u32 },
    /// The player picked an item up from the floor (it will be minted).
    Picked { item: ItemId, amount: u32 },
    /// The player emptied a grave.
    GraveLooted { grave_id: u32 },
    /// The wielded blade broke (it will be burned).
    BladeBroke,
    /// The player drank a potion.
    PotionDrunk { healed: u32 },
    /// The player opened a locked door with a key (the key will be burned).
    DoorOpened { x: i32, y: i32 },
    /// The player walked into a locked door without a key.
    DoorLocked { x: i32, y: i32 },
    /// The player took the stairs; a new depth was generated.
    Descended { depth: u32 },
    /// The player regained 1 HP.
    Regenerated,
    /// A monster appeared at (x, y).
    Spawned { monster: u32, x: i32, y: i32 },
    /// The player died at (x, y).
    Died { x: i32, y: i32 },
    /// The player took the way up and left the dungeon: the dive is over, nothing is lost.
    Surfaced,
}

/// The result of one `DiveSim::step`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct StepEvents {
    /// False when the dive was already over: the move was ignored and nothing changed.
    pub applied: bool,
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MonsterView {
    /// Stable slot index on this depth (events refer to it); a respawn may reuse a dead slot.
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub hp: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct GraveMarker {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub looted: bool,
    pub items: Vec<(ItemId, u32)>,
}

/// Everything the browser needs to draw the current state of a dive.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiveView {
    pub depth: u32,
    pub turn: u32,
    pub width: u32,
    pub height: u32,
    /// `width * height` tiles, row-major (`tiles[y * width + x]`): WALL, FLOOR, STAIRS, ...
    pub tiles: Vec<u8>,
    /// The way up on this depth (where the dive arrived): `MOVE_SURFACE` works only here.
    pub exit_x: i32,
    pub exit_y: i32,
    pub x: i32,
    pub y: i32,
    pub hp: i32,
    pub max_hp: i32,
    /// Live monsters only.
    pub monsters: Vec<MonsterView>,
    pub items: Vec<FloorItem>,
    pub graves: Vec<GraveMarker>,
    /// What the player carries right now (loadout + finds - used up).
    pub inventory: Vec<(ItemId, u32)>,
    /// Hits left on the wielded blade (0 when no blade is carried).
    pub blade_durability: u32,
    pub kills: u32,
    pub moves_applied: u32,
    pub over: bool,
    pub died: bool,
    pub returned: bool,
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

/// Removes up to `amount` of `id` from a list (entries that reach 0 are dropped); returns how
/// many were removed.
fn remove_item(list: &mut Vec<(ItemId, u32)>, id: ItemId, amount: u32) -> u32 {
    let Some(i) = list.iter().position(|e| e.0 == id) else { return 0 };
    let taken = list[i].1.min(amount);
    list[i].1 -= taken;
    if list[i].1 == 0 {
        list.remove(i);
    }
    taken
}

pub fn count_item(list: &[(ItemId, u32)], id: ItemId) -> u32 {
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
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
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
    pub id: u32,
    pub items: Vec<(ItemId, u32)>,
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
    /// The vault door, on `rules::VAULT_DEPTH` only.
    pub door: Option<(i32, i32)>,
}

impl Level {
    pub fn tile(&self, x: i32, y: i32) -> u8 {
        tile_at(&self.tiles, x, y)
    }

    /// Floor or stairs (a closed door is not passable).
    pub fn passable(&self, x: i32, y: i32) -> bool {
        matches!(self.tile(x, y), FLOOR | STAIRS)
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

/// The given cell if it is floor, otherwise the nearest floor cell (ring by ring, fixed order).
fn snap_to_floor(tiles: &[u8; CELLS], x: i32, y: i32) -> (i32, i32) {
    if tile_at(tiles, x, y) == FLOOR {
        return (x, y);
    }
    for r in 1..W.max(H) {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dy.abs() != r {
                    continue;
                }
                if tile_at(tiles, x + dx, y + dy) == FLOOR {
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
/// player, which is what lets a grave written by one dive be found by another. Graves of other
/// depths in `graves` are ignored.
pub fn generate_level(seed: u64, depth: u32, graves: &[Grave]) -> Level {
    let mut rng = level_rng(seed, depth);
    let mut tiles = [WALL; CELLS];
    // (x, y, w, h)
    let mut rooms: Vec<(i32, i32, i32, i32)> = Vec::new();

    for _ in 0..rules::ROOM_ATTEMPTS {
        let w = rules::ROOM_W_MIN + rng.below(rules::ROOM_W_SPREAD) as i32;
        let h = rules::ROOM_H_MIN + rng.below(rules::ROOM_H_SPREAD) as i32;
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

    let vault = if depth == rules::VAULT_DEPTH { carve_vault(&mut tiles, &mut rng) } else { None };

    let mut grave_spots = Vec::new();
    for g in graves.iter().filter(|g| g.depth == depth) {
        let (gx, gy) = snap_to_floor(&tiles, i32::from(g.x), i32::from(g.y));
        grave_spots.push(GraveSpot { x: gx, y: gy, id: g.id, items: g.items.clone(), looted: false });
    }

    let mut monsters = Vec::with_capacity(rules::MONSTERS_PER_LEVEL);
    for _ in 0..rules::MONSTERS_PER_LEVEL {
        if let Some((mx, my)) = free_spot(&tiles, &monsters, &mut rng, start, rules::MONSTER_MIN_DIST) {
            monsters.push(Monster { x: mx, y: my, hp: rules::monster_hp(depth), alive: true });
        }
    }

    let mut items = Vec::new();
    let d = depth.min(rules::DEPTH_CAP);
    let allowed = || rules::FLOOR_DROPS.iter().filter(|row| row.2 <= depth);
    let total: u32 = allowed().map(|row| row.1).sum();
    for _ in 0..rules::ITEMS_PER_LEVEL {
        let (ix, iy) = random_floor(&tiles, &mut rng);
        let mut roll = rng.below(total);
        let mut item = ITEM_GOLD;
        for &(id, weight, _) in allowed() {
            if roll < weight {
                item = id;
                break;
            }
            roll -= weight;
        }
        let amount = if item == ITEM_GOLD { 1 + rng.below(rules::FLOOR_GOLD_SPREAD) + d / 2 } else { 1 };
        if (ix, iy) != start {
            items.push(FloorItem { x: ix, y: iy, item, amount });
        }
    }
    let door = vault.map(|(door, (gx, gy))| {
        items.push(FloorItem { x: gx, y: gy, item: ITEM_GEM, amount: rules::VAULT_GEMS });
        door
    });

    Level { depth, tiles, start, stairs, monsters, items, graves: grave_spots, door }
}

/// Carves the vault into solid rock: a `VAULT_SIZE` square room whose wall ring stays intact
/// except for one blue door, plus a straight corridor from the door to the nearest open tile.
/// Returns (door, centre of the room). Tries every placement from a seeded starting point, so it
/// only fails on a map with no solid `VAULT_SIZE + 2` block (never seen; see the tests).
fn carve_vault(tiles: &mut [u8; CELLS], rng: &mut Rng) -> Option<((i32, i32), (i32, i32))> {
    let s = rules::VAULT_SIZE;
    let span = s + 2;
    let (nx, ny) = (W - span + 1, H - span + 1);
    let n = (nx * ny) as u32;
    let first = rng.below(n);
    let side0 = rng.below(4);
    for k in 0..n {
        let i = ((first + k) % n) as i32;
        let (bx, by) = (i % nx, i / nx);
        let solid = (by..by + span).all(|y| (bx..bx + span).all(|x| tile_at(tiles, x, y) == WALL));
        if !solid {
            continue;
        }
        let mid = s / 2 + 1;
        // (door, outward step) for the top, right, bottom and left walls.
        let sides = [
            ((bx + mid, by), (0, -1)),
            ((bx + span - 1, by + mid), (1, 0)),
            ((bx + mid, by + span - 1), (0, 1)),
            ((bx, by + mid), (-1, 0)),
        ];
        for j in 0..4u32 {
            let ((dx, dy), (ox, oy)) = sides[((side0 + j) % 4) as usize];
            if dx < 1 || dy < 1 || dx > W - 2 || dy > H - 2 {
                continue;
            }
            // Walk outward through rock until an open tile; the map border stops the search.
            let (mut cx, mut cy) = (dx + ox, dy + oy);
            let mut len = 0;
            let reached = loop {
                if cx < 1 || cy < 1 || cx > W - 2 || cy > H - 2 {
                    break false;
                }
                if tile_at(tiles, cx, cy) != WALL {
                    break true;
                }
                cx += ox;
                cy += oy;
                len += 1;
            };
            if !reached {
                continue;
            }
            for step in 1..=len {
                set_tile(tiles, dx + ox * step, dy + oy * step, FLOOR);
            }
            for y in by + 1..by + 1 + s {
                for x in bx + 1..bx + 1 + s {
                    set_tile(tiles, x, y, FLOOR);
                }
            }
            set_tile(tiles, dx, dy, DOOR_BLUE);
            return Some(((dx, dy), (bx + mid, by + mid)));
        }
    }
    None
}

/// Where the grave of an expired dive goes: a seeded spot (snapped to floor on each day's map).
pub fn expired_grave_spot(seed: u64, dive_id: u64) -> (u8, u8) {
    let mut rng = Rng::new(seed ^ dive_id.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0x6578_7069_7265_6421);
    ((1 + rng.below(W as u32 - 2)) as u8, (1 + rng.below(H as u32 - 2)) as u8)
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

/// Step-by-step dive. The browser calls `step` once per key press; the contract's `run_dive`
/// is the same `step` loop over the whole log, so both always agree.
#[derive(Clone, Debug)]
pub struct DiveSim {
    seed: u64,
    /// Graves handed in through `DiveEntry::graves` (read lazily per depth by `step`).
    preloaded: Vec<Grave>,
    level: Level,
    depth: u32,
    x: i32,
    y: i32,
    hp: i32,
    carried: Vec<(ItemId, u32)>,
    picked: Vec<(ItemId, u32)>,
    looted: Vec<u32>,
    burned: Vec<(ItemId, u32)>,
    rng: Rng,
    turn: u32,
    died: bool,
    death_pos: Option<(u8, u8)>,
    moves_applied: u32,
    kills: u32,
    blade_wear: u32,
    blades_broken: u32,
    potions_used: u32,
    keys_used: u32,
    vault_opened: bool,
    /// The player took the way up (`MOVE_SURFACE`).
    returned: bool,
    /// When set (by `run_dive`), steps do not record events: same rules, less work on chain.
    quiet: bool,
    events: Vec<Event>,
}

impl DiveSim {
    /// Starts a dive whose graves come from `entry.graves`.
    pub fn new(seed: u64, entry: &DiveEntry) -> DiveSim {
        let mut src: &[Grave] = &entry.graves;
        let mut sim = DiveSim::with_source(seed, entry, &mut src);
        sim.preloaded = entry.graves.clone();
        sim
    }

    /// Starts a dive whose graves come from `src` (`entry.graves` is ignored). Use `step_with`
    /// with the same source afterwards.
    pub fn with_source(seed: u64, entry: &DiveEntry, src: &mut impl GraveSource) -> DiveSim {
        let depth = entry.start_depth.max(1);
        let level = generate_level(seed, depth, &src.graves_at(depth));
        let mut carried = Vec::new();
        for &(id, amount) in &entry.inventory {
            add_item(&mut carried, id, amount);
        }
        let (x, y) = level.start;
        let mut sim = DiveSim {
            seed,
            preloaded: Vec::new(),
            level,
            depth,
            x,
            y,
            hp: 0,
            carried,
            picked: Vec::new(),
            looted: Vec::new(),
            burned: Vec::new(),
            rng: Rng::new(seed ^ 0xA076_1D64_78BD_642F),
            turn: 0,
            died: false,
            death_pos: None,
            moves_applied: 0,
            kills: 0,
            blade_wear: 0,
            blades_broken: 0,
            potions_used: 0,
            keys_used: 0,
            vault_opened: false,
            returned: false,
            quiet: false,
            events: Vec::new(),
        };
        sim.hp = sim.max_hp();
        sim
    }

    /// True once the player is dead, has returned to the surface, or `MAX_MOVES` moves were
    /// applied; further steps are ignored.
    pub fn is_over(&self) -> bool {
        self.died || self.returned || self.moves_applied >= MAX_MOVES
    }

    pub fn is_dead(&self) -> bool {
        self.died
    }

    /// Applies one move, reading new depths' graves from `DiveEntry::graves`.
    pub fn step(&mut self, mv: u8) -> StepEvents {
        let pre = core::mem::take(&mut self.preloaded);
        let mut src: &[Grave] = &pre;
        let out = self.step_with(mv, &mut src);
        self.preloaded = pre;
        out
    }

    /// Applies one move, reading new depths' graves from `src`.
    pub fn step_with(&mut self, mv: u8, src: &mut impl GraveSource) -> StepEvents {
        if self.is_over() {
            return StepEvents { applied: false, events: Vec::new() };
        }
        self.moves_applied += 1;
        self.turn += 1;

        if mv == MOVE_SURFACE && (self.x, self.y) == self.level.start {
            // The way up: the dive ends here, before the monsters move, and everything carried is
            // banked.
            self.returned = true;
            self.emit(Event::Surfaced);
            return self.take_events();
        }
        let (dx, dy) = move_delta(mv);
        if mv == MOVE_POTION {
            self.drink();
        } else if dx != 0 || dy != 0 {
            let (nx, ny) = (self.x + dx, self.y + dy);
            if self.level.tile(nx, ny) == DOOR_BLUE {
                self.try_door(nx, ny);
            }
            if let Some(i) = self.level.monster_at(nx, ny) {
                self.player_attack(i);
            } else if self.level.passable(nx, ny) {
                self.x = nx;
                self.y = ny;
                self.emit(Event::Moved { x: nx, y: ny });
                self.pick_up();
                if self.level.tile(nx, ny) == STAIRS {
                    self.descend(src);
                    self.end_of_turn();
                    return self.take_events();
                }
            } else if self.level.tile(nx, ny) != DOOR_BLUE {
                // Into a wall: no-op, the move still counts.
                self.emit(Event::Bumped { x: nx, y: ny });
            }
        }

        self.monsters_act();
        if !self.died {
            self.end_of_turn();
        }
        self.take_events()
    }

    fn emit(&mut self, e: Event) {
        if !self.quiet {
            self.events.push(e);
        }
    }

    fn take_events(&mut self) -> StepEvents {
        StepEvents { applied: true, events: core::mem::take(&mut self.events) }
    }

    pub fn max_hp(&self) -> i32 {
        rules::BASE_HP + rules::GEM_HP * count_item(&self.carried, ITEM_GEM).min(rules::GEM_CAP) as i32
    }

    fn attack_bonus(&self) -> i32 {
        if count_item(&self.carried, ITEM_BLADE) > 0 {
            rules::BLADE_ATTACK
        } else {
            0
        }
    }

    /// Burns one carried `id` (used up or broken): it leaves the pack and goes to `burned`.
    fn use_up(&mut self, id: ItemId) -> bool {
        if remove_item(&mut self.carried, id, 1) == 1 {
            add_item(&mut self.burned, id, 1);
            true
        } else {
            false
        }
    }

    fn drink(&mut self) {
        let max = self.max_hp();
        if self.hp >= max || !self.use_up(ITEM_POTION) {
            return;
        }
        let healed = rules::POTION_HEAL.min(max - self.hp);
        self.hp += healed;
        self.potions_used += 1;
        self.emit(Event::PotionDrunk { healed: healed as u32 });
    }

    /// Walking into the vault door: a blue key opens it (and is used up), otherwise it is locked.
    fn try_door(&mut self, x: i32, y: i32) {
        if self.use_up(ITEM_BLUE_KEY) {
            set_tile(&mut self.level.tiles, x, y, FLOOR);
            self.keys_used += 1;
            self.vault_opened = true;
            self.emit(Event::DoorOpened { x, y });
        } else {
            self.emit(Event::DoorLocked { x, y });
        }
    }

    /// Hits left on the wielded blade, 0 without a blade.
    fn blade_durability(&self) -> u32 {
        if count_item(&self.carried, ITEM_BLADE) > 0 {
            rules::BLADE_DURABILITY - self.blade_wear
        } else {
            0
        }
    }

    fn armor(&self) -> i32 {
        rules::PLATE_ARMOR * count_item(&self.carried, ITEM_PLATE).min(rules::PLATE_CAP) as i32
    }

    fn player_attack(&mut self, i: usize) {
        let dmg = 1 + self.rng.below(rules::PLAYER_DAMAGE_SPREAD) as i32 + self.attack_bonus();
        if count_item(&self.carried, ITEM_BLADE) > 0 {
            self.blade_wear += 1;
            if self.blade_wear >= rules::BLADE_DURABILITY {
                // This hit still lands with the blade; then it breaks and the next one is drawn.
                self.use_up(ITEM_BLADE);
                self.blade_wear = 0;
                self.blades_broken += 1;
                self.emit(Event::BladeBroke);
            }
        }
        let m = &mut self.level.monsters[i];
        m.hp -= dmg;
        let killed = m.hp <= 0;
        let (mx, my) = (m.x, m.y);
        if killed {
            m.alive = false;
        }
        self.emit(Event::Attacked { monster: i as u32, damage: dmg as u32, killed });
        if killed {
            self.kills += 1;
            if self.rng.below(100) < rules::MONSTER_GOLD_DROP_PCT {
                let amount = 1 + self.rng.below(rules::MONSTER_GOLD_SPREAD);
                self.level.items.push(FloorItem { x: mx, y: my, item: ITEM_GOLD, amount });
                self.emit(Event::Dropped { x: mx, y: my, item: ITEM_GOLD, amount });
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
                self.emit(Event::Picked { item: it.item, amount: it.amount });
            } else {
                i += 1;
            }
        }
        for gi in 0..self.level.graves.len() {
            // A dive empties at most GRAVES_LOOTED_PER_DIVE graves; later ones stay where they are.
            if self.looted.len() >= rules::GRAVES_LOOTED_PER_DIVE as usize {
                break;
            }
            let spot = &mut self.level.graves[gi];
            if spot.looted || spot.x != self.x || spot.y != self.y {
                continue;
            }
            spot.looted = true;
            let id = spot.id;
            let items = spot.items.clone();
            self.looted.push(id);
            for (item, amount) in items {
                add_item(&mut self.carried, item, amount);
            }
            self.emit(Event::GraveLooted { grave_id: id });
        }
    }

    fn descend(&mut self, src: &mut impl GraveSource) {
        self.depth += 1;
        self.level = generate_level(self.seed, self.depth, &src.graves_at(self.depth));
        self.x = self.level.start.0;
        self.y = self.level.start.1;
        self.emit(Event::Descended { depth: self.depth });
    }

    fn monsters_act(&mut self) {
        for i in 0..self.level.monsters.len() {
            let m = &self.level.monsters[i];
            if !m.alive {
                continue;
            }
            if (m.x - self.x).abs() + (m.y - self.y).abs() == 1 {
                if self.rng.below(100) < rules::MONSTER_HIT_PCT {
                    // Armor softens a hit but every hit does at least 1 damage.
                    let raw = 1 + self.rng.below(rules::monster_damage_spread(self.depth)) as i32;
                    let dmg = (raw - self.armor()).max(1);
                    self.hp -= dmg;
                    self.emit(Event::Hurt { monster: i as u32, damage: dmg as u32 });
                    if self.hp <= 0 {
                        self.hp = 0;
                        self.died = true;
                        self.death_pos = Some((self.x as u8, self.y as u8));
                        self.emit(Event::Died { x: self.x, y: self.y });
                        return;
                    }
                } else {
                    self.emit(Event::Missed { monster: i as u32 });
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
        if self.turn % rules::REGEN_EVERY == 0 && self.hp < self.max_hp() {
            self.hp += 1;
            self.emit(Event::Regenerated);
        }
        if self.turn % rules::RESPAWN_EVERY == 0 {
            self.respawn();
        }
    }

    fn respawn(&mut self) {
        let alive = self.level.monsters.iter().filter(|m| m.alive).count();
        if alive >= rules::MONSTERS_PER_LEVEL {
            return;
        }
        let spot = free_spot(
            &self.level.tiles,
            &self.level.monsters,
            &mut self.rng,
            (self.x, self.y),
            rules::RESPAWN_MIN_DIST,
        );
        if let Some((x, y)) = spot {
            let m = Monster { x, y, hp: rules::monster_hp(self.depth), alive: true };
            let slot = if let Some(slot) = self.level.monsters.iter().position(|m| !m.alive) {
                self.level.monsters[slot] = m;
                slot
            } else {
                self.level.monsters.push(m);
                self.level.monsters.len() - 1
            };
            self.emit(Event::Spawned { monster: slot as u32, x, y });
        }
    }

    /// The current state, for drawing.
    pub fn view(&self) -> DiveView {
        DiveView {
            depth: self.depth,
            turn: self.turn,
            width: W as u32,
            height: H as u32,
            tiles: self.level.tiles.to_vec(),
            exit_x: self.level.start.0,
            exit_y: self.level.start.1,
            x: self.x,
            y: self.y,
            hp: self.hp,
            max_hp: self.max_hp(),
            monsters: self
                .level
                .monsters
                .iter()
                .enumerate()
                .filter(|(_, m)| m.alive)
                .map(|(i, m)| MonsterView { id: i as u32, x: m.x, y: m.y, hp: m.hp })
                .collect(),
            items: self.level.items.clone(),
            graves: self
                .level
                .graves
                .iter()
                .map(|g| GraveMarker { id: g.id, x: g.x, y: g.y, looted: g.looted, items: g.items.clone() })
                .collect(),
            inventory: self.carried.clone(),
            blade_durability: self.blade_durability(),
            kills: self.kills,
            moves_applied: self.moves_applied,
            over: self.is_over(),
            died: self.died,
            returned: self.returned,
        }
    }

    /// What the contract would apply if the log ended here. Unless the player died or returned,
    /// that is a quit: a death where the player stands.
    pub fn outcome(&self) -> DiveOutcome {
        let quit = !self.died && !self.returned;
        let dead = self.died || quit;
        let mut dropped = Vec::new();
        let mut burned = self.burned.clone();
        if dead {
            for &(id, amount) in &self.carried {
                let toll = if id == ITEM_GOLD { amount / rules::DEATH_TOLL_GOLD_DIV } else { 0 };
                add_item(&mut burned, id, toll);
                add_item(&mut dropped, id, amount - toll);
            }
        }
        DiveOutcome {
            final_depth: self.depth,
            died: dead,
            returned: self.returned,
            quit,
            moves_applied: self.moves_applied,
            picked: self.picked.clone(),
            looted_graves: self.looted.clone(),
            dropped_into_grave: dropped,
            burned,
            death_pos: if quit { Some((self.x as u8, self.y as u8)) } else { self.death_pos },
            hp_left: self.hp.max(0) as u32,
            kills: self.kills,
            blades_broken: self.blades_broken,
            potions_used: self.potions_used,
            keys_used: self.keys_used,
            vault_opened: self.vault_opened,
        }
    }
}

/// Re-runs a whole dive with graves from `entry.graves`: the `DiveSim::step` loop.
pub fn run_dive(seed: u64, entry: &DiveEntry, moves: &[u8]) -> DiveOutcome {
    let mut src: &[Grave] = &entry.graves;
    run_dive_with(seed, entry, moves, &mut src)
}

/// Re-runs a whole dive with graves from `src`: what the contract executes on `settle_dive`.
pub fn run_dive_with(seed: u64, entry: &DiveEntry, moves: &[u8], src: &mut impl GraveSource) -> DiveOutcome {
    let mut sim = DiveSim::with_source(seed, entry, src);
    sim.quiet = true;
    for &mv in moves {
        if !sim.step_with(mv, src).applied {
            break;
        }
    }
    sim.outcome()
}

#[cfg(test)]
mod tests;
