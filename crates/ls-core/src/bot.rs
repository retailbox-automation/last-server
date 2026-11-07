//! Native move-log generator for tests and the gas benchmark (feature `bot`).
//!
//! Not part of the rules: it plays the same `Dive` stepper a real player would, so the logs
//! it produces walk through rooms, fight, rest, pick things up and take stairs, instead of
//! bumping one wall thousands of times.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Fights what is adjacent, rests when hurt, collects items and graves, then descends
    /// (down to `BOT_MAX_DEPTH`).
    Survivor,
    /// Goes for unlooted graves first, otherwise plays like `Survivor`.
    Looter,
    /// Walks up to the nearest monster and never fights back, so it dies.
    Doomed,
    /// Runs for the stairs every turn (fighting only what is adjacent): the most new maps per
    /// move, i.e. the expensive end of plausible logs for the gas benchmark.
    Rusher,
}

/// Plays `n` moves with the given style and returns the log. If the player dies early the
/// rest of the log is padded with waits, which the rules ignore after death.
pub fn generate_moves(seed: u64, entry: &DiveEntry, n: usize, style: Style) -> Vec<u8> {
    let mut dive = Dive::new(seed, entry);
    let mut log = Vec::with_capacity(n);
    while log.len() < n {
        let mv = if dive.is_dead() { MOVE_WAIT } else { choose(&dive, style) };
        log.push(mv);
        dive.step(mv);
    }
    log
}

fn choose(d: &Dive<'_>, style: Style) -> u8 {
    let monster_here = |x: i32, y: i32| d.level.monster_at(x, y).is_some();
    if style == Style::Doomed {
        let adjacent = DIRS.iter().any(|&(_, dx, dy)| monster_here(d.x + dx, d.y + dy));
        if adjacent {
            return MOVE_WAIT;
        }
        // Walk next to the nearest monster (the step that would enter it is never taken).
        return first_step(d, |x, y| DIRS.iter().any(|&(_, dx, dy)| monster_here(x + dx, y + dy)))
            .filter(|&mv| {
                let (_, dx, dy) = DIRS[mv as usize];
                !monster_here(d.x + dx, d.y + dy)
            })
            .unwrap_or(MOVE_WAIT);
    }
    // 1. Fight an adjacent monster.
    for (mv, dx, dy) in DIRS {
        if d.level.monster_at(d.x + dx, d.y + dy).is_some() {
            return mv;
        }
    }
    if style == Style::Rusher {
        let stairs = d.level.stairs;
        return first_step(d, |x, y| (x, y) == stairs).unwrap_or(MOVE_WAIT);
    }
    let max = d.max_hp();
    // 2. Rest when hurt.
    if d.hp * 10 < max * 8 {
        return MOVE_WAIT;
    }
    // 3. Graves and items.
    let grave_here = |x: i32, y: i32| d.level.graves.iter().any(|g| !g.looted && g.x == x && g.y == y);
    let item_here = |x: i32, y: i32| d.level.items.iter().any(|it| it.x == x && it.y == y);
    if style == Style::Looter {
        if let Some(mv) = first_step(d, grave_here) {
            return mv;
        }
    }
    if let Some(mv) = first_step(d, |x, y| grave_here(x, y) || item_here(x, y)) {
        return mv;
    }
    // 4. Descend only at full health, and not past the depth where monsters outgrow the bot.
    if d.hp >= max && d.depth < BOT_MAX_DEPTH {
        let stairs = d.level.stairs;
        if let Some(mv) = first_step(d, |x, y| (x, y) == stairs) {
            return mv;
        }
    }
    MOVE_WAIT
}

/// The bot stops taking stairs here; deeper, its simple tactics die within a few hundred moves.
pub const BOT_MAX_DEPTH: u32 = 20;

const DIRS: [(u8, i32, i32); 4] =
    [(MOVE_UP, 0, -1), (MOVE_DOWN, 0, 1), (MOVE_LEFT, -1, 0), (MOVE_RIGHT, 1, 0)];

/// BFS over passable tiles from the player; returns the first move toward the nearest cell
/// matching `target` (stairs are only entered when they are the target).
fn first_step(d: &Dive<'_>, target: impl Fn(i32, i32) -> bool) -> Option<u8> {
    const NONE: u8 = 255;
    let mut first = [NONE; CELLS];
    let mut seen = [false; CELLS];
    let mut queue: Vec<(i32, i32)> = Vec::with_capacity(CELLS);
    let idx = |x: i32, y: i32| (y * W + x) as usize;
    seen[idx(d.x, d.y)] = true;
    queue.push((d.x, d.y));
    let mut head = 0;
    while head < queue.len() {
        let (x, y) = queue[head];
        head += 1;
        for (mv, dx, dy) in DIRS {
            let (nx, ny) = (x + dx, y + dy);
            if !d.level.passable(nx, ny) || seen[idx(nx, ny)] {
                continue;
            }
            seen[idx(nx, ny)] = true;
            let step = if (x, y) == (d.x, d.y) { mv } else { first[idx(x, y)] };
            if target(nx, ny) {
                return Some(step);
            }
            if d.level.tile(nx, ny) == STAIRS {
                continue;
            }
            first[idx(nx, ny)] = step;
            queue.push((nx, ny));
        }
    }
    None
}
