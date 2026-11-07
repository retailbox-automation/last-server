//! Native move-log generator for tests and the gas benchmark (feature `bot`).
//!
//! Not part of the rules: it plays the same `DiveSim` stepper a real player would, so the logs
//! it produces walk through rooms, fight, rest, pick things up and take stairs, instead of
//! bumping one wall thousands of times.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Fights what is adjacent, drinks a potion when low, rests when hurt, collects items and
    /// graves, opens the vault when it carries a blue key, then descends (down to `BOT_MAX_DEPTH`).
    Survivor,
    /// Goes for unlooted graves first, otherwise plays like `Survivor`.
    Looter,
    /// Walks up to the nearest monster and never fights back, so it dies.
    Doomed,
    /// Runs for the stairs every turn (fighting only what is adjacent, drinking when low): the
    /// most new maps per move, i.e. the expensive end of plausible logs for the gas benchmark.
    Rusher,
    /// Plays like `Survivor` without ever stepping on a grave, and once it reaches the given depth
    /// plays like `Doomed`: leaves a grave at that depth without looting the ones above it.
    DieAt(u32),
}

/// Plays `n` moves with the given style and returns the log. If the dive ends early the rest of
/// the log is padded with waits, which the rules ignore.
pub fn generate_moves(seed: u64, entry: &DiveEntry, n: usize, style: Style) -> Vec<u8> {
    let mut dive = DiveSim::new(seed, entry);
    let mut log = Vec::with_capacity(n);
    while log.len() < n {
        let mv = if dive.is_over() { MOVE_WAIT } else { choose(&dive, style) };
        log.push(mv);
        dive.step(mv);
    }
    log
}

fn choose(d: &DiveSim, style: Style) -> u8 {
    let monster_here = |x: i32, y: i32| d.level.monster_at(x, y).is_some();
    let doomed = match style {
        Style::Doomed => true,
        Style::DieAt(depth) => d.depth >= depth,
        _ => false,
    };
    if doomed {
        let adjacent = DIRS.iter().any(|&(_, dx, dy)| monster_here(d.x + dx, d.y + dy));
        if adjacent {
            return MOVE_WAIT;
        }
        // Walk next to the nearest monster (the step that would enter it is never taken).
        let near_monster = |x: i32, y: i32| DIRS.iter().any(|&(_, dx, dy)| monster_here(x + dx, y + dy));
        let avoid_graves = matches!(style, Style::DieAt(_));
        // A grave may block the only corridor: then walk over it after all.
        return first_step(d, avoid_graves, near_monster)
            .or_else(|| first_step(d, false, near_monster))
            .filter(|&mv| {
                let (_, dx, dy) = DIRS[mv as usize];
                !monster_here(d.x + dx, d.y + dy)
            })
            .unwrap_or(MOVE_WAIT);
    }
    let avoid_graves = matches!(style, Style::DieAt(_));
    let max = d.max_hp();
    let potions = count_item(&d.carried, ITEM_POTION);
    let adjacent = DIRS.iter().any(|&(_, dx, dy)| monster_here(d.x + dx, d.y + dy));
    // 1. Low in a fight: drink. Otherwise fight an adjacent monster.
    if adjacent && potions > 0 && d.hp * 10 < max * 4 {
        return MOVE_POTION;
    }
    for (mv, dx, dy) in DIRS {
        if monster_here(d.x + dx, d.y + dy) {
            return mv;
        }
    }
    if style == Style::Rusher {
        let stairs = d.level.stairs;
        return first_step(d, false, |x, y| (x, y) == stairs).unwrap_or(MOVE_WAIT);
    }
    // 2. Hurt: drink when quite low, otherwise rest.
    if d.hp * 10 < max * 8 {
        return if potions > 0 && d.hp * 10 < max * 5 { MOVE_POTION } else { MOVE_WAIT };
    }
    // 3. Graves, items, and the vault door when carrying a blue key.
    let grave_here = |x: i32, y: i32| d.level.graves.iter().any(|g| !g.looted && g.x == x && g.y == y);
    let item_here = |x: i32, y: i32| d.level.items.iter().any(|it| it.x == x && it.y == y);
    if style == Style::Looter {
        if let Some(mv) = first_step(d, false, grave_here) {
            return mv;
        }
    }
    let wants = |x: i32, y: i32| (!avoid_graves && grave_here(x, y)) || item_here(x, y);
    if let Some(mv) = first_step(d, avoid_graves, wants) {
        return mv;
    }
    if count_item(&d.carried, ITEM_BLUE_KEY) > 0 {
        if let Some(door) = d.level.door.filter(|&(x, y)| d.level.tile(x, y) == DOOR_BLUE) {
            if let Some(mv) = first_step(d, avoid_graves, |x, y| (x, y) == door) {
                return mv;
            }
        }
    }
    // 4. Descend only at full health, and not past the depth where monsters outgrow the bot.
    let max_depth = if let Style::DieAt(depth) = style { depth } else { BOT_MAX_DEPTH };
    if d.hp >= max && d.depth < max_depth {
        let stairs = d.level.stairs;
        let to_stairs = |x: i32, y: i32| (x, y) == stairs;
        if let Some(mv) = first_step(d, avoid_graves, to_stairs).or_else(|| first_step(d, false, to_stairs)) {
            return mv;
        }
    }
    MOVE_WAIT
}

/// The bot stops taking stairs here; deeper, its simple tactics die within a few hundred moves.
pub const BOT_MAX_DEPTH: u32 = 15;

const DIRS: [(u8, i32, i32); 4] =
    [(MOVE_UP, 0, -1), (MOVE_DOWN, 0, 1), (MOVE_LEFT, -1, 0), (MOVE_RIGHT, 1, 0)];

/// BFS over passable tiles from the player; returns the first move toward the nearest cell
/// matching `target` (a target may be a closed door; stairs are only entered when they are the
/// target; with `avoid_graves`, unlooted grave tiles are never entered).
fn first_step(d: &DiveSim, avoid_graves: bool, target: impl Fn(i32, i32) -> bool) -> Option<u8> {
    const NONE: u8 = 255;
    let mut first = [NONE; CELLS];
    let mut seen = [false; CELLS];
    let mut queue: Vec<(i32, i32)> = Vec::with_capacity(CELLS);
    let idx = |x: i32, y: i32| (y * W + x) as usize;
    let blocked = |x: i32, y: i32| avoid_graves && d.level.graves.iter().any(|g| !g.looted && g.x == x && g.y == y);
    seen[idx(d.x, d.y)] = true;
    queue.push((d.x, d.y));
    let mut head = 0;
    while head < queue.len() {
        let (x, y) = queue[head];
        head += 1;
        for (mv, dx, dy) in DIRS {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= W || ny >= H || seen[idx(nx, ny)] {
                continue;
            }
            let step = if (x, y) == (d.x, d.y) { mv } else { first[idx(x, y)] };
            if target(nx, ny) && !blocked(nx, ny) && (d.level.passable(nx, ny) || d.level.tile(nx, ny) == DOOR_BLUE) {
                return Some(step);
            }
            if !d.level.passable(nx, ny) || blocked(nx, ny) {
                continue;
            }
            seen[idx(nx, ny)] = true;
            if d.level.tile(nx, ny) == STAIRS {
                continue;
            }
            first[idx(nx, ny)] = step;
            queue.push((nx, ny));
        }
    }
    None
}
