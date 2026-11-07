//! Every number of the Last Server rules, in one file.
//!
//! `RULES_HASH` is the sha256 of this file's bytes (build.rs), reported by the contract's
//! `core_version` view and by the browser, so anyone can check that both run these numbers.
//! The engine that applies them is `lib.rs`; `CORE_HASH` covers both files.

/// Map size of every depth, in tiles.
pub const W: i32 = 32;
pub const H: i32 = 32;

/// Depth every dive starts at.
pub const START_DEPTH: u32 = 1;
/// Longest move log a dive accepts; moves beyond it are ignored (and the contract rejects
/// longer logs outright).
pub const MAX_MOVES: u32 = 4000;
/// Depth beyond which monsters stop getting stronger (keeps i32 math far from overflow).
pub const DEPTH_CAP: u32 = 1000;

// ---- level generation ------------------------------------------------------------------------

pub const ROOM_ATTEMPTS: u32 = 14;
/// Room width is `ROOM_W_MIN + roll(ROOM_W_SPREAD)`, height likewise.
pub const ROOM_W_MIN: i32 = 4;
pub const ROOM_W_SPREAD: u32 = 6;
pub const ROOM_H_MIN: i32 = 3;
pub const ROOM_H_SPREAD: u32 = 5;
pub const MONSTERS_PER_LEVEL: usize = 8;
pub const ITEMS_PER_LEVEL: u32 = 6;
/// Monsters are placed at least this far (Manhattan) from the level start.
pub const MONSTER_MIN_DIST: i32 = 6;

// ---- player ----------------------------------------------------------------------------------

pub const BASE_HP: i32 = 20;
/// The player regains 1 HP every this many turns.
pub const REGEN_EVERY: u32 = 5;

// ---- monsters --------------------------------------------------------------------------------

/// A dead monster is replaced every this many turns (if the level has fewer than the maximum).
pub const RESPAWN_EVERY: u32 = 25;
/// Respawned monsters appear at least this far (Manhattan) from the player.
pub const RESPAWN_MIN_DIST: i32 = 8;
/// Chance in percent that an adjacent monster hits the player on its turn.
pub const MONSTER_HIT_PCT: u32 = 60;
/// Chance in percent that a killed monster drops gold.
pub const MONSTER_GOLD_DROP_PCT: u32 = 50;
/// Gold dropped by a monster: `1 + roll(MONSTER_GOLD_SPREAD)`.
pub const MONSTER_GOLD_SPREAD: u32 = 3;

/// Monster hit points at a depth.
pub fn monster_hp(depth: u32) -> i32 {
    3 + depth.min(DEPTH_CAP) as i32 / 2
}

/// Raw monster damage is `1 + roll(monster_damage_spread(depth))`, before armor.
pub fn monster_damage_spread(depth: u32) -> u32 {
    2 + depth.min(DEPTH_CAP) / 6
}

/// Player damage is `1 + roll(PLAYER_DAMAGE_SPREAD) + weapon bonus`.
pub const PLAYER_DAMAGE_SPREAD: u32 = 4;

// ---- items -----------------------------------------------------------------------------------

/// Attack bonus per carried blade, and how many blades count.
pub const BLADE_ATTACK: i32 = 1;
pub const BLADE_CAP: u32 = 3;
/// Armor per carried plate (subtracted from each hit, every hit still does 1), and how many count.
pub const PLATE_ARMOR: i32 = 1;
pub const PLATE_CAP: u32 = 2;
/// Max HP per carried gem, and how many count.
pub const GEM_HP: i32 = 2;
pub const GEM_CAP: u32 = 5;

/// Floor drop table: a roll in `0..100` below the first threshold gives gold, and so on.
pub const DROP_GOLD_BELOW: u32 = 55;
pub const DROP_BLADE_BELOW: u32 = 75;
pub const DROP_PLATE_BELOW: u32 = 93;
/// Gold on the floor: `1 + roll(FLOOR_GOLD_SPREAD) + depth / 2`.
pub const FLOOR_GOLD_SPREAD: u32 = 5;

// ---- death -----------------------------------------------------------------------------------

/// On death one gold in this many is burned (the rest goes into the grave).
pub const DEATH_TOLL_GOLD_DIV: u32 = 10;
