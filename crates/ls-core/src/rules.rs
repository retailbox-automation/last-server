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
/// The player regains 1 HP every this many turns. Slow on purpose: potions heal at once,
/// resting costs moves (the GAME-LOOP draft said "no regen"; with none, a 300-move dive ends by
/// attrition within the first depths, so it is slow instead; tune with the fast-forward bots).
pub const REGEN_EVERY: u32 = 8;

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

/// Player damage is `1 + roll(PLAYER_DAMAGE_SPREAD) + blade bonus`.
pub const PLAYER_DAMAGE_SPREAD: u32 = 4;

// ---- items -----------------------------------------------------------------------------------

/// Attack bonus while a blade is wielded (one at a time; spare blades wait in the pack).
pub const BLADE_ATTACK: i32 = 2;
/// Hits a blade survives inside one dive. Wear does not leave the dive: a blade that comes back
/// is as good as new, a blade that breaks is burned when the dive settles.
pub const BLADE_DURABILITY: u32 = 20;
/// Armor per carried plate (subtracted from each hit, every hit still does 1), and how many count.
pub const PLATE_ARMOR: i32 = 1;
pub const PLATE_CAP: u32 = 2;
/// Max HP per carried gem, and how many count.
pub const GEM_HP: i32 = 2;
pub const GEM_CAP: u32 = 5;
/// HP a potion restores (never above max HP). A potion is only drunk when it heals something.
pub const POTION_HEAL: i32 = 8;

/// Floor drop table: `(item, weight, first depth it can appear at)`. Each floor item rolls
/// `0..sum of weights` over the rows allowed at that depth. Gems never lie on the floor: they
/// are only found in the vault.
pub const FLOOR_DROPS: [(u32, u32, u32); 5] = [
    (crate::ITEM_GOLD, 50, 1),
    (crate::ITEM_BLADE, 14, 1),
    (crate::ITEM_PLATE, 12, 1),
    (crate::ITEM_POTION, 16, 1),
    (crate::ITEM_BLUE_KEY, 8, 5),
];
/// Gold on the floor: `1 + roll(FLOOR_GOLD_SPREAD) + depth / 2`.
pub const FLOOR_GOLD_SPREAD: u32 = 5;

// ---- the vault -------------------------------------------------------------------------------

/// The one depth with a vault: a walled room whose only way in is a blue door.
pub const VAULT_DEPTH: u32 = 7;
/// Inner size of the vault (square).
pub const VAULT_SIZE: i32 = 3;
/// Gems waiting in the vault on every visit.
pub const VAULT_GEMS: u32 = 1;

// ---- death -----------------------------------------------------------------------------------

/// On death one gold in this many is burned (the rest goes into the grave).
pub const DEATH_TOLL_GOLD_DIV: u32 = 10;
/// Graves kept per depth; a new grave beyond this makes the oldest one crumble (its items burn).
pub const GRAVES_PER_DEPTH: u32 = 10;
/// Graves one dive can empty. Beyond this the player walks over graves without taking them (they
/// stay for the next player). It bounds what one `settle_dive` writes and logs: NEAR allows 16 KB
/// of logs per call and every looted grave adds a log row (the contract tests check the worst case).
pub const GRAVES_LOOTED_PER_DIVE: u32 = 50;

// ---- dive lifetime (applied by the contract) -------------------------------------------------

/// An open dive can be expired by anyone once its day is over AND at least this long after it
/// started, so a dive started just before 00:00 UTC is not expired at 00:00:01 while its owner
/// is still playing.
pub const EXPIRE_GRACE_SECONDS: u64 = 4 * 3600;
