extern crate std;

use super::bot::{generate_moves, moves_to_quit_at, safe_grave_spots, Style};
use super::*;
use std::vec;

fn starter_entry() -> DiveEntry {
    DiveEntry { start_depth: 1, inventory: vec![(ITEM_GOLD, 25), (ITEM_BLADE, 1)], graves: vec![] }
}

#[test]
fn same_inputs_same_outcome() {
    let entry = starter_entry();
    let moves = generate_moves(7, &entry, 1000, Style::Survivor);
    let a = run_dive(7, &entry, &moves);
    let b = run_dive(7, &entry, &moves);
    assert_eq!(a, b);
    assert!(a.returned, "the bot walks back up before its 1000 moves run out: {a:?}");
}

#[test]
fn different_seed_different_map() {
    let a = generate_level(1, 1, &[]);
    let b = generate_level(2, 1, &[]);
    assert_ne!(a.tiles, b.tiles);
    // Same seed and depth: same map, monsters and items.
    assert_eq!(generate_level(1, 1, &[]), a);
    // Same seed, next depth: a different map.
    assert_ne!(generate_level(1, 2, &[]).tiles, a.tiles);
}

#[test]
fn level_is_sane() {
    for seed in 0..200u64 {
        let l = generate_level(seed, 1 + (seed % 7) as u32, &[]);
        assert_eq!(l.tile(l.start.0, l.start.1), FLOOR, "seed {seed}");
        assert_eq!(l.tile(l.stairs.0, l.stairs.1), STAIRS, "seed {seed}");
        for x in 0..W {
            assert_eq!(l.tile(x, 0), WALL);
            assert_eq!(l.tile(x, H - 1), WALL);
        }
        assert!(!l.monsters.is_empty(), "seed {seed}");
        for m in &l.monsters {
            assert_eq!(l.tile(m.x, m.y), FLOOR);
        }
    }
}

#[test]
fn bot_log_of_3000_moves_goes_deep_and_survives() {
    let mut outcomes = vec![];
    // The gas benchmark dives with seeds 1..=3 from fresh accounts (empty inventory).
    let fresh = DiveEntry { start_depth: 1, ..DiveEntry::default() };
    let cases = [1u64, 2, 3, 11, 12, 13, 21, 22, 23]
        .into_iter()
        .map(|s| (s, starter_entry()))
        .chain([1u64, 2, 3].into_iter().map(|s| (s, fresh.clone())));
    for (seed, entry) in cases {
        let moves = generate_moves(seed, &entry, 3000, Style::Survivor);
        let out = run_dive(seed, &entry, &moves);
        std::println!(
            "seed {seed}: depth {} died {} applied {} kills {} hp {}",
            out.final_depth,
            out.died,
            out.moves_applied,
            out.kills,
            out.hp_left
        );
        outcomes.push((seed, out));
    }
    for (seed, out) in outcomes {
        assert!(out.final_depth > 1, "seed {seed} stayed at depth 1");
        assert!(out.returned && !out.died, "seed {seed} did not return from depth {}", out.final_depth);
        assert!(out.moves_applied > 2500 && out.moves_applied <= 3000, "seed {seed}: {} moves", out.moves_applied);
        assert!(!out.picked.is_empty());
        assert!(out.kills > 0);
    }
}

#[test]
fn random_move_bytes_never_panic() {
    let mut rng = Rng::new(0xDEAD_BEEF);
    for i in 0..1000u32 {
        let len = rng.below(3001) as usize;
        let moves: Vec<u8> = (0..len).map(|_| rng.next_u64() as u8).collect();
        let entry = DiveEntry {
            start_depth: rng.below(40),
            inventory: vec![(ITEM_GOLD, rng.below(1000)), (ITEM_PLATE, rng.below(4))],
            graves: vec![Grave {
                id: i,
                depth: 1 + rng.below(3),
                x: rng.below(256) as u8,
                y: rng.below(256) as u8,
                items: vec![(ITEM_GEM, 1)],
            }],
        };
        let out = run_dive(u64::from(i) * 7919, &entry, &moves);
        assert!(out.moves_applied as usize <= moves.len());
        assert!(out.died != out.returned, "case {i}: {out:?}");
        if out.quit {
            assert_eq!(out.moves_applied as usize, moves.len());
        }
    }
}

#[test]
fn death_leaves_grave_and_next_player_loots_it() {
    let seed = 42;
    let alice_entry = DiveEntry {
        start_depth: 1,
        inventory: vec![(ITEM_GOLD, 57), (ITEM_BLADE, 2), (ITEM_GEM, 1)],
        graves: vec![],
    };
    let doomed = generate_moves(seed, &alice_entry, 400, Style::Doomed);
    let dead = run_dive(seed, &alice_entry, &doomed);
    assert!(dead.died, "a player who never fights back must die");
    assert!(dead.moves_applied < 400);
    let (gx, gy) = dead.death_pos.expect("death position");
    // Death toll: one gold in ten is burned, everything else carried goes into the grave.
    let mut carried = alice_entry.inventory.clone();
    for &(id, n) in &dead.picked {
        add_item(&mut carried, id, n);
    }
    let gold = count_item(&carried, ITEM_GOLD);
    assert_eq!(dead.burned, vec![(ITEM_GOLD, gold / 10)]);
    let mut expected = vec![];
    for &(id, n) in &carried {
        add_item(&mut expected, id, if id == ITEM_GOLD { n - gold / 10 } else { n });
    }
    assert_eq!(dead.dropped_into_grave, expected);

    let grave = Grave { id: 77, depth: dead.final_depth, x: gx, y: gy, items: dead.dropped_into_grave };
    let bob_entry = DiveEntry { start_depth: 1, inventory: vec![], graves: vec![grave] };
    let looter = generate_moves(seed, &bob_entry, 400, Style::Looter);
    let bob = run_dive(seed, &bob_entry, &looter);
    assert_eq!(bob.looted_graves, vec![77]);
}

#[test]
fn grave_on_a_wall_snaps_to_floor() {
    let base = generate_level(5, 1, &[]);
    // (0, 0) is always a wall.
    let g = Grave { id: 1, depth: 1, x: 0, y: 0, items: vec![(ITEM_GOLD, 1)] };
    let l = generate_level(5, 1, &[g]);
    assert_eq!(l.graves.len(), 1);
    assert_ne!(base.tile(l.graves[0].x, l.graves[0].y), WALL);
}

#[test]
fn walls_are_no_ops_that_count() {
    let entry = DiveEntry::default();
    let mut dive = DiveSim::new(9, &entry);
    dive.level.monsters.clear();
    let mut xs = vec![];
    let mut bumped = false;
    for _ in 0..40 {
        let ev = dive.step(MOVE_LEFT);
        assert!(ev.applied);
        bumped |= ev.events.iter().any(|e| matches!(e, Event::Bumped { .. }));
        xs.push(dive.x);
    }
    // The player walks left until a wall, then stays put while the moves keep counting.
    let last = *xs.last().unwrap();
    assert!(xs.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] - 1));
    assert_eq!(dive.level.tile(last - 1, dive.y), WALL);
    assert!(bumped);
    assert_eq!(dive.outcome().moves_applied, 40);
}

#[test]
fn item_names_round_trip() {
    for id in ITEM_IDS {
        assert_eq!(item_id(item_name(id).unwrap()), Some(id));
    }
    assert_eq!(item_name(0), None);
    assert_eq!(item_id("sword"), None);
}

/// Random logs and graves for the stepping tests.
fn random_case(rng: &mut Rng, i: u32) -> (u64, DiveEntry, Vec<u8>) {
    let len = rng.below(1500) as usize;
    let moves: Vec<u8> = (0..len).map(|_| rng.below(7) as u8).collect();
    let graves = (0..rng.below(6))
        .map(|g| Grave {
            id: g,
            depth: 1 + rng.below(9),
            x: rng.below(32) as u8,
            y: rng.below(32) as u8,
            items: vec![(ITEM_GOLD, 1 + rng.below(9)), (ITEM_BLADE, rng.below(2))],
        })
        .collect();
    let entry = DiveEntry {
        start_depth: 1 + rng.below(8),
        inventory: vec![
            (ITEM_GOLD, rng.below(50)),
            (ITEM_BLADE, rng.below(3)),
            (ITEM_PLATE, rng.below(2)),
            (ITEM_POTION, rng.below(3)),
            (ITEM_BLUE_KEY, rng.below(2)),
        ],
        graves,
    };
    (u64::from(i).wrapping_mul(0x9E37_79B9) ^ rng.next_u64(), entry, moves)
}

#[test]
fn run_dive_equals_manual_stepping() {
    let mut rng = Rng::new(0x5EED);
    let mut deaths = 0;
    for i in 0..300u32 {
        let (seed, entry, moves) = random_case(&mut rng, i);
        let mut sim = DiveSim::new(seed, &entry);
        let mut applied = 0;
        for &mv in &moves {
            let ev = sim.step(mv);
            if ev.applied {
                applied += 1;
            } else {
                assert!(sim.is_over());
                assert!(ev.events.is_empty());
            }
        }
        let out = run_dive(seed, &entry, &moves);
        assert_eq!(sim.outcome(), out, "case {i}");
        assert_eq!(out.moves_applied, applied);
        deaths += u32::from(out.died);
    }
    // Bot logs too (they reach deeper depths, loot graves and fight a lot).
    for seed in 0..40u64 {
        let entry = starter_entry();
        for style in [Style::Survivor, Style::Rusher, Style::Doomed, Style::Looter] {
            let moves = generate_moves(seed, &entry, 700, style);
            let mut sim = DiveSim::new(seed, &entry);
            for &mv in &moves {
                sim.step(mv);
            }
            assert_eq!(sim.outcome(), run_dive(seed, &entry, &moves), "seed {seed} {style:?}");
        }
    }
    std::println!("random cases that died: {deaths}/300");
}

/// A lazy source must give the same outcome as the preloaded list, and is asked once per depth.
#[test]
fn lazy_graves_match_preloaded() {
    struct Counting<'a> {
        all: &'a [Grave],
        asked: Vec<u32>,
    }
    impl GraveSource for Counting<'_> {
        fn graves_at(&mut self, depth: u32) -> Vec<Grave> {
            self.asked.push(depth);
            self.all.iter().filter(|g| g.depth == depth).cloned().collect()
        }
    }
    let mut rng = Rng::new(0xFACE);
    for i in 0..200u32 {
        let (seed, entry, moves) = random_case(&mut rng, i);
        let mut src = Counting { all: &entry.graves, asked: vec![] };
        let lazy = run_dive_with(seed, &DiveEntry { graves: vec![], ..entry.clone() }, &moves, &mut src);
        assert_eq!(lazy, run_dive(seed, &entry, &moves), "case {i}");
        let expected: Vec<u32> = (entry.start_depth.max(1)..=lazy.final_depth).collect();
        assert_eq!(src.asked, expected, "case {i}: one read per depth entered");
    }
}

#[test]
fn view_tracks_the_dive() {
    let entry = starter_entry();
    let moves = generate_moves(3, &entry, 400, Style::Survivor);
    let mut sim = DiveSim::new(3, &entry);
    let v0 = sim.view();
    assert_eq!((v0.width, v0.height), (W as u32, H as u32));
    assert_eq!(v0.tiles.len(), (W * H) as usize);
    assert_eq!(v0.depth, 1);
    assert_eq!(v0.hp, v0.max_hp);
    assert_eq!(v0.inventory, entry.inventory);
    assert_eq!(v0.tiles[(v0.y * W + v0.x) as usize], FLOOR);
    for &mv in &moves {
        let ev = sim.step(mv);
        let v = sim.view();
        assert_eq!(v.moves_applied, sim.outcome().moves_applied);
        for e in &ev.events {
            if let Event::Descended { depth } = e {
                assert_eq!(*depth, v.depth);
            }
        }
        assert!(v.monsters.iter().all(|m| m.hp > 0));
        assert!(v.tiles[(v.y * W + v.x) as usize] != WALL);
    }
    assert!(sim.view().over && sim.view().returned, "the survivor walks back up within its 400 moves");
}

#[test]
fn move_limit_ends_the_dive() {
    let entry = DiveEntry::default();
    let mut sim = DiveSim::new(1, &entry);
    sim.level.monsters.clear();
    // A wall-bumping log that never dies: the stepper stops at MAX_MOVES.
    let moves = vec![MOVE_WAIT; MAX_MOVES as usize + 10];
    let mut applied = 0;
    for &mv in &moves {
        if sim.step(mv).applied {
            applied += 1;
        }
    }
    assert_eq!(applied, MAX_MOVES);
    assert!(sim.is_over());
    assert!(!sim.is_dead());
    let out = run_dive(1, &entry, &moves);
    assert!(out.moves_applied <= MAX_MOVES);
    assert!(sim.outcome().quit, "a dive that runs out of moves inside the dungeon is a quit");
}

#[cfg(feature = "serde")]
#[test]
fn types_round_trip_through_json() {
    extern crate serde_json;
    let entry = starter_entry();
    let moves = generate_moves(5, &entry, 300, Style::Survivor);
    let mut sim = DiveSim::new(5, &entry);
    for &mv in &moves {
        let ev = sim.step(mv);
        let j = serde_json::to_string(&ev).unwrap();
        assert_eq!(serde_json::from_str::<StepEvents>(&j).unwrap(), ev);
    }
    let v = sim.view();
    assert_eq!(serde_json::from_str::<DiveView>(&serde_json::to_string(&v).unwrap()).unwrap(), v);
    let o = sim.outcome();
    assert_eq!(serde_json::from_str::<DiveOutcome>(&serde_json::to_string(&o).unwrap()).unwrap(), o);
    let e = serde_json::to_value(&Event::Attacked { monster: 2, damage: 3, killed: true }).unwrap();
    assert_eq!(e, serde_json::json!({"kind": "attacked", "monster": 2, "damage": 3, "killed": true}));
}

/// Cells reachable from the level start through passable tiles (a closed door blocks).
fn reachable(l: &Level) -> std::vec::Vec<bool> {
    let mut seen = vec![false; (W * H) as usize];
    let mut queue = vec![l.start];
    seen[(l.start.1 * W + l.start.0) as usize] = true;
    while let Some((x, y)) = queue.pop() {
        for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
            let (nx, ny) = (x + dx, y + dy);
            if l.passable(nx, ny) && !seen[(ny * W + nx) as usize] {
                seen[(ny * W + nx) as usize] = true;
                queue.push((nx, ny));
            }
        }
    }
    seen
}

#[test]
fn one_vault_at_depth_7_behind_the_blue_door() {
    for seed in 0..2000u64 {
        let l = generate_level(seed, rules::VAULT_DEPTH, &[]);
        let doors: std::vec::Vec<usize> = (0..l.tiles.len()).filter(|&i| l.tiles[i] == DOOR_BLUE).collect();
        assert_eq!(doors.len(), 1, "seed {seed}");
        let (dx, dy) = l.door.expect("door");
        assert_eq!(doors[0], (dy * W + dx) as usize);
        let gems: std::vec::Vec<&FloorItem> = l.items.iter().filter(|i| i.item == ITEM_GEM).collect();
        assert_eq!(gems.len(), 1, "seed {seed}");
        assert_eq!(gems[0].amount, rules::VAULT_GEMS);
        // Closed door: the gem is out of reach; open door: the gem and the stairs are reachable.
        let closed = reachable(&l);
        assert!(!closed[(gems[0].y * W + gems[0].x) as usize], "seed {seed}: gem reachable without the key");
        assert!(closed[(l.stairs.1 * W + l.stairs.0) as usize], "seed {seed}: stairs cut off");
        let mut open = l.clone();
        open.tiles[doors[0]] = FLOOR;
        assert!(reachable(&open)[(gems[0].y * W + gems[0].x) as usize], "seed {seed}: vault sealed");
    }
}

#[test]
fn keys_deep_gems_only_in_the_vault() {
    let mut keys_by_depth = [0u32; 13];
    for seed in 0..300u64 {
        for depth in 1..=12u32 {
            let l = generate_level(seed, depth, &[]);
            if depth != rules::VAULT_DEPTH {
                assert!(l.door.is_none());
                assert!(l.tiles.iter().all(|&t| t != DOOR_BLUE));
                assert!(l.items.iter().all(|i| i.item != ITEM_GEM), "seed {seed} depth {depth}: gem on the floor");
            }
            for it in &l.items {
                assert!(ITEM_IDS.contains(&it.item));
                if it.item == ITEM_BLUE_KEY {
                    keys_by_depth[depth as usize] += 1;
                }
            }
        }
    }
    assert_eq!(&keys_by_depth[1..5], &[0, 0, 0, 0], "keys above depth 5");
    assert!(keys_by_depth[5..].iter().all(|&k| k > 0), "{keys_by_depth:?}");
}

/// A sim with no monsters except, optionally, one sturdy monster east of the player.
fn quiet_sim(entry: &DiveEntry) -> DiveSim {
    let mut sim = DiveSim::new(11, entry);
    sim.level.monsters.clear();
    sim
}

#[test]
fn blade_breaks_after_its_durability_and_is_burned() {
    let entry = DiveEntry { start_depth: 1, inventory: vec![(ITEM_BLADE, 2)], graves: vec![] };
    let mut sim = quiet_sim(&entry);
    sim.hp = 1_000_000;
    // Put a monster that cannot die in a free cell next to the player.
    let (mx, my) = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .map(|(dx, dy)| (sim.x + dx, sim.y + dy))
        .find(|&(x, y)| sim.level.passable(x, y))
        .unwrap();
    let mv = match (mx - sim.x, my - sim.y) {
        (1, 0) => MOVE_RIGHT,
        (-1, 0) => MOVE_LEFT,
        (0, 1) => MOVE_DOWN,
        _ => MOVE_UP,
    };
    sim.level.monsters.push(Monster { x: mx, y: my, hp: 1_000_000, alive: true });
    assert_eq!(sim.view().blade_durability, rules::BLADE_DURABILITY);
    for hit in 1..=rules::BLADE_DURABILITY {
        let ev = sim.step(mv);
        let broke = ev.events.contains(&Event::BladeBroke);
        assert_eq!(broke, hit == rules::BLADE_DURABILITY, "hit {hit}");
    }
    // One blade left, fresh; the broken one is burned.
    assert_eq!(count_item(&sim.carried, ITEM_BLADE), 1);
    assert_eq!(sim.view().blade_durability, rules::BLADE_DURABILITY);
    let out = sim.outcome();
    assert_eq!(out.blades_broken, 1);
    assert_eq!(out.burned, vec![(ITEM_BLADE, 1)]);
    for _ in 0..rules::BLADE_DURABILITY {
        sim.step(mv);
    }
    assert_eq!(sim.view().blade_durability, 0);
    assert_eq!(sim.outcome().burned, vec![(ITEM_BLADE, 2)]);
    assert!(sim.view().inventory.iter().all(|e| e.0 != ITEM_BLADE));
}

#[test]
fn potion_heals_only_when_hurt_and_is_used_up() {
    let entry = DiveEntry { start_depth: 1, inventory: vec![(ITEM_POTION, 2)], graves: vec![] };
    let mut sim = quiet_sim(&entry);
    // Full HP: drinking is a wait.
    let ev = sim.step(MOVE_POTION);
    assert!(ev.applied && ev.events.is_empty());
    assert_eq!(count_item(&sim.carried, ITEM_POTION), 2);
    sim.hp = 3;
    let ev = sim.step(MOVE_POTION);
    assert_eq!(ev.events, vec![Event::PotionDrunk { healed: rules::POTION_HEAL as u32 }]);
    assert_eq!(sim.hp, 3 + rules::POTION_HEAL);
    sim.hp = sim.max_hp() - 2;
    sim.step(MOVE_POTION);
    assert_eq!(sim.hp, sim.max_hp(), "never above max");
    sim.hp = 1;
    let ev = sim.step(MOVE_POTION);
    assert!(ev.events.iter().all(|e| !matches!(e, Event::PotionDrunk { .. })), "no potion left");
    let out = sim.outcome();
    assert_eq!(out.potions_used, 2);
    assert_eq!(out.burned, vec![(ITEM_POTION, 2)]);
}

/// Puts the player on the corridor tile just outside the vault door; returns the move into it.
fn stand_at_door(sim: &mut DiveSim) -> u8 {
    let (dx, dy) = sim.level.door.expect("vault depth has a door");
    for (mv, ox, oy) in [(MOVE_DOWN, 0, -1), (MOVE_UP, 0, 1), (MOVE_RIGHT, -1, 0), (MOVE_LEFT, 1, 0)] {
        let (x, y) = (dx + ox, dy + oy);
        if sim.level.passable(x, y) && reachable(&sim.level)[(y * W + x) as usize] {
            sim.x = x;
            sim.y = y;
            return mv;
        }
    }
    panic!("no corridor next to the door");
}

#[test]
fn blue_key_opens_the_vault_and_the_gem_is_found() {
    for seed in 0..50u64 {
        // No key: the door stays shut and the player does not move.
        let entry = DiveEntry { start_depth: rules::VAULT_DEPTH, inventory: vec![], graves: vec![] };
        let mut sim = DiveSim::new(seed, &entry);
        sim.level.monsters.clear();
        let mv = stand_at_door(&mut sim);
        let before = (sim.x, sim.y);
        let ev = sim.step(mv);
        assert!(ev.events.iter().any(|e| matches!(e, Event::DoorLocked { .. })), "seed {seed}");
        assert_eq!((sim.x, sim.y), before);
        assert!(!sim.outcome().vault_opened);

        // With a key: the door opens, the key burns, the player walks in and takes the gem.
        let entry = DiveEntry { start_depth: rules::VAULT_DEPTH, inventory: vec![(ITEM_BLUE_KEY, 1)], graves: vec![] };
        let mut sim = DiveSim::new(seed, &entry);
        sim.level.monsters.clear();
        let mv = stand_at_door(&mut sim);
        let ev = sim.step(mv);
        assert!(ev.events.iter().any(|e| matches!(e, Event::DoorOpened { .. })), "seed {seed}");
        assert_eq!((sim.x, sim.y), sim.level.door.unwrap());
        let gem = sim.level.items.iter().find(|i| i.item == ITEM_GEM).unwrap().clone();
        // Walk to the gem (straight in: the door is in the middle of a vault wall).
        for _ in 0..rules::VAULT_SIZE {
            if (sim.x, sim.y) == (gem.x, gem.y) {
                break;
            }
            sim.step(mv);
        }
        let out = sim.outcome();
        assert!(out.vault_opened && out.keys_used == 1, "seed {seed}");
        assert_eq!(out.burned, vec![(ITEM_BLUE_KEY, 1)]);
        // (A random floor item may also have landed inside the vault.)
        assert_eq!(count_item(&out.picked, ITEM_GEM), rules::VAULT_GEMS, "seed {seed}: {:?}", out.picked);
        assert_eq!(sim.max_hp(), rules::BASE_HP + rules::GEM_HP);
    }
}

#[test]
fn bots_carry_a_key_to_the_vault() {
    let mut opened = 0;
    let n = 40u64;
    for seed in 0..n {
        let entry = DiveEntry { start_depth: 1, inventory: vec![(ITEM_BLUE_KEY, 1), (ITEM_BLADE, 1)], graves: vec![] };
        let moves = generate_moves(seed, &entry, 3000, Style::Survivor);
        let out = run_dive(seed, &entry, &moves);
        if out.vault_opened {
            opened += 1;
            assert!(count_item(&out.picked, ITEM_GEM) >= 1, "seed {seed}: opened but no gem");
        }
        assert!(out.keys_used <= 1 + count_item(&out.picked, ITEM_BLUE_KEY));
    }
    std::println!("vault opened in {opened}/{n} dives");
    assert!(opened * 10 >= n * 8, "vault opened in only {opened}/{n} dives");
}

/// Nothing is burned or dropped that the player did not have: per item,
/// loadout + picked + looted - burned is what is banked (alive) or exactly the grave (dead).
#[test]
fn every_item_is_accounted_for() {
    let mut rng = Rng::new(0xACC0);
    let mut cases = vec![];
    for i in 0..300u32 {
        cases.push(random_case(&mut rng, i));
    }
    for seed in 0..30u64 {
        let entry = DiveEntry {
            start_depth: 1,
            inventory: vec![(ITEM_GOLD, 40), (ITEM_BLADE, 2), (ITEM_POTION, 2), (ITEM_BLUE_KEY, 1)],
            graves: vec![],
        };
        for style in [Style::Survivor, Style::Rusher, Style::Doomed, Style::DieAt(4)] {
            let moves = generate_moves(seed, &entry, 1500, style);
            cases.push((seed, entry.clone(), moves));
        }
    }
    let (mut died, mut broke, mut drank) = (0, 0, 0);
    for (n, (seed, entry, moves)) in cases.iter().enumerate() {
        let out = run_dive(*seed, entry, moves);
        for id in ITEM_IDS {
            let mut have = i64::from(count_item(&entry.inventory, id)) + i64::from(count_item(&out.picked, id));
            for g in entry.graves.iter().filter(|g| out.looted_graves.contains(&g.id)) {
                have += i64::from(count_item(&g.items, id));
            }
            let left = have - i64::from(count_item(&out.burned, id));
            assert!(left >= 0, "case {n} item {id}: burned more than carried");
            if out.died {
                assert_eq!(left, i64::from(count_item(&out.dropped_into_grave, id)), "case {n} item {id}");
            } else {
                assert!(out.dropped_into_grave.is_empty());
            }
        }
        assert_eq!(count_item(&out.burned, ITEM_BLADE), out.blades_broken, "case {n}");
        assert_eq!(count_item(&out.burned, ITEM_POTION), out.potions_used, "case {n}");
        assert_eq!(count_item(&out.burned, ITEM_BLUE_KEY), out.keys_used, "case {n}");
        assert_eq!(out.vault_opened, out.keys_used > 0, "case {n}");
        died += u32::from(out.died);
        broke += out.blades_broken;
        drank += out.potions_used;
    }
    std::println!("cases {}: died {died}, blades broken {broke}, potions drunk {drank}", cases.len());
    assert!(died > 0 && broke > 0 && drank > 0, "the accounting paths were not all exercised");
}

#[test]
fn expired_grave_spot_is_seeded_and_inside() {
    let a = expired_grave_spot(5, 1);
    assert_eq!(a, expired_grave_spot(5, 1));
    assert_ne!(a, expired_grave_spot(5, 2));
    for d in 0..500u64 {
        let (x, y) = expired_grave_spot(d * 31, d);
        assert!(x >= 1 && i32::from(x) < W - 1 && y >= 1 && i32::from(y) < H - 1);
    }
}

#[test]
fn die_at_leaves_a_grave_at_its_depth_without_looting() {
    let mut graves = vec![];
    for (i, depth) in (1..=6u32).enumerate() {
        graves.push(Grave { id: i as u32, depth, x: 10, y: 10, items: vec![(ITEM_GOLD, 3)] });
    }
    let entry = DiveEntry { start_depth: 1, inventory: vec![(ITEM_GOLD, 1)], graves };
    let (mut at_target, mut died) = (0, 0);
    for seed in 0..20u64 {
        for target in [1u32, 3, 6] {
            let moves = generate_moves(seed, &entry, 3000, Style::DieAt(target));
            let out = run_dive(seed, &entry, &moves);
            // Graves are walked around in the doomed phase; on the way down a grave that blocks
            // the only corridor to the stairs is crossed (and looted).
            if target == 1 {
                assert!(out.looted_graves.is_empty(), "seed {seed}: looted {:?}", out.looted_graves);
            }
            died += u32::from(out.died);
            at_target += u32::from(out.died && out.final_depth == target);
        }
    }
    std::println!("DieAt: died {died}/60, at the target depth {at_target}/60");
    assert!(at_target >= 45, "died at the target depth in {at_target}/60 dives");
}

/// Up to `GRAVES_PER_DEPTH` graves on the tiles a well-equipped rusher walks over on every depth:
/// the dive empties exactly `GRAVES_LOOTED_PER_DIVE` of them, in walking order, and leaves the rest.
/// The graves hold only gold, which changes nothing in the dungeon, and looting draws no random
/// numbers, so the same log walks the same path with and without them.
#[test]
fn a_dive_loots_at_most_graves_looted_per_dive() {
    let cap = rules::GRAVES_LOOTED_PER_DIVE as usize;
    let entry_of = |graves: Vec<Grave>| DiveEntry {
        start_depth: 1,
        inventory: vec![(ITEM_PLATE, 2), (ITEM_GEM, 5), (ITEM_BLADE, 10), (ITEM_POTION, 300)],
        graves,
    };
    let (mut over_cap, mut most) = (0, 0);
    for seed in 0..10u64 {
        let plain = entry_of(vec![]);
        let moves = generate_moves(seed, &plain, MAX_MOVES as usize, Style::Rusher);
        let base = run_dive(seed, &plain, &moves);
        // Tiles stepped on (not the stairs), per depth, first visits only.
        let mut sim = DiveSim::new(seed, &plain);
        let mut graves: Vec<Grave> = vec![];
        for &mv in &moves {
            let depth = sim.depth;
            let ev = sim.step(mv);
            if ev.events.iter().any(|e| matches!(e, Event::Descended { .. })) {
                continue;
            }
            let Some(&Event::Moved { x, y }) = ev.events.iter().find(|e| matches!(e, Event::Moved { .. })) else {
                continue;
            };
            let here = graves.iter().filter(|g| g.depth == depth);
            if here.clone().count() as u32 >= rules::GRAVES_PER_DEPTH || here.clone().any(|g| (i32::from(g.x), i32::from(g.y)) == (x, y)) {
                continue;
            }
            assert_eq!(generate_level(seed, depth, &[]).tile(x, y), FLOOR);
            let id = graves.len() as u32;
            graves.push(Grave { id, depth, x: x as u8, y: y as u8, items: vec![(ITEM_GOLD, 1)] });
        }
        most = most.max(graves.len());
        let entry = entry_of(graves.clone());
        let out = run_dive(seed, &entry, &moves);
        std::println!(
            "seed {seed}: depth {} died {} graves on its path {} looted {}",
            out.final_depth,
            out.died,
            graves.len(),
            out.looted_graves.len()
        );
        assert_eq!((out.final_depth, out.died, out.moves_applied), (base.final_depth, base.died, base.moves_applied));
        let expected: Vec<u32> = graves.iter().take(cap).map(|g| g.id).collect();
        assert_eq!(out.looted_graves, expected, "seed {seed}");
        // Stepping agrees with run_dive; on the last depth a grave is marked looted only if it was.
        let mut sim = DiveSim::new(seed, &entry);
        for &mv in &moves {
            sim.step(mv);
        }
        assert_eq!(sim.outcome(), out, "seed {seed}");
        for g in sim.view().graves {
            assert_eq!(g.looted, out.looted_graves.contains(&g.id), "seed {seed} grave {}", g.id);
        }
        over_cap += usize::from(graves.len() > cap);
    }
    std::println!("most graves on one dive's path: {most}; dives over the cap: {over_cap}/10");
    assert!(over_cap >= 5, "the cap was reached in only {over_cap}/10 dives");
}

/// Once a dive has emptied as many graves as it can, the looting bots move on instead of walking
/// between the graves left behind until the log runs out. `GRAVES_PER_DEPTH` graves (gold only) on
/// depths 1..=6, more than the cap in total.
#[test]
fn bots_move_on_after_graves_looted_per_dive() {
    let cap = rules::GRAVES_LOOTED_PER_DIVE as usize;
    let (mut stuck, mut reached) = (std::vec::Vec::new(), 0);
    for seed in 0..6u64 {
        let mut graves = vec![];
        for depth in 1..=6u32 {
            let l = generate_level(seed, depth, &[]);
            let spots = (0..W * H)
                .map(|i| (i % W, i / W))
                .filter(|&(x, y)| l.tile(x, y) == FLOOR && (x, y) != l.start)
                .step_by(7)
                .take(rules::GRAVES_PER_DEPTH as usize);
            for (x, y) in spots {
                let id = graves.len() as u32;
                graves.push(Grave { id, depth, x: x as u8, y: y as u8, items: vec![(ITEM_GOLD, 1)] });
            }
        }
        assert!(graves.len() > cap);
        let entry = DiveEntry {
            start_depth: 1,
            inventory: vec![(ITEM_PLATE, 2), (ITEM_GEM, 5), (ITEM_BLADE, 10), (ITEM_POTION, 50)],
            graves,
        };
        for style in [Style::Looter, Style::Survivor] {
            let moves = generate_moves(seed, &entry, MAX_MOVES as usize, style);
            let out = run_dive(seed, &entry, &moves);
            std::println!("seed {seed} {style:?}: looted {} depth {} died {}", out.looted_graves.len(), out.final_depth, out.died);
            assert!(out.looted_graves.len() <= cap);
            if out.looted_graves.len() == cap {
                reached += 1;
                if out.final_depth <= 6 && !out.died {
                    stuck.push((seed, style));
                }
            }
        }
    }
    assert!(stuck.is_empty(), "bots that reached the cap and stayed among the graves: {stuck:?}");
    assert!(reached >= 4, "only {reached}/12 dives reached the cap");
}

/// A log cut one move before the death it leads to banks nothing: it is a quit, which counts as a
/// death where the player stood (grave and death toll as for any death).
#[test]
fn a_log_that_stops_before_death_is_a_quit_not_a_return() {
    for seed in 0..20u64 {
        let entry = DiveEntry { start_depth: 1, inventory: vec![(ITEM_GOLD, 57), (ITEM_BLADE, 2)], graves: vec![] };
        let doomed = generate_moves(seed, &entry, 2000, Style::Doomed);
        let dead = run_dive(seed, &entry, &doomed);
        assert!(dead.died && !dead.quit && !dead.returned, "seed {seed}: {dead:?}");
        let cut = &doomed[..dead.moves_applied as usize - 1];
        let out = run_dive(seed, &entry, cut);
        let mut sim = DiveSim::new(seed, &entry);
        for &mv in cut {
            sim.step(mv);
        }
        assert!(!sim.is_dead() && !sim.is_over(), "seed {seed}: alive after the cut log");
        assert!(out.died && out.quit && !out.returned, "seed {seed}: {out:?}");
        assert_eq!(out.death_pos, Some((sim.x as u8, sim.y as u8)), "seed {seed}");
        assert!(out.hp_left > 0);
        let mut grave = vec![];
        let mut toll = vec![];
        for &(id, n) in &sim.view().inventory {
            let t = if id == ITEM_GOLD { n / rules::DEATH_TOLL_GOLD_DIV } else { 0 };
            add_item(&mut grave, id, n - t);
            add_item(&mut toll, id, t);
        }
        assert_eq!(out.dropped_into_grave, grave, "seed {seed}");
        assert_eq!(count_item(&out.burned, ITEM_GOLD), count_item(&toll, ITEM_GOLD), "seed {seed}");
    }
}

/// Walking back to where the dive arrived and taking the way up banks everything: the dive ends
/// there (later moves are ignored), nothing is dropped. Anywhere else the move is a wait. An empty
/// log is a quit at the start: the loadout, minus the toll, becomes the grave.
#[test]
fn the_way_up_returns_to_the_surface() {
    let entry = DiveEntry { start_depth: 1, inventory: vec![(ITEM_GOLD, 30), (ITEM_PLATE, 1)], graves: vec![] };
    let empty = run_dive(9, &entry, &[]);
    let start = generate_level(9, 1, &[]).start;
    assert!(empty.died && empty.quit && !empty.returned, "{empty:?}");
    assert_eq!(empty.death_pos, Some((start.0 as u8, start.1 as u8)));
    assert_eq!(empty.dropped_into_grave, vec![(ITEM_GOLD, 27), (ITEM_PLATE, 1)]);
    assert_eq!(empty.burned, vec![(ITEM_GOLD, 3)]);

    let mut sim = quiet_sim(&entry);
    let (sx, sy) = (sim.x, sim.y);
    assert_eq!((sim.view().exit_x, sim.view().exit_y), (sx, sy));
    let (off, back) = [(MOVE_RIGHT, MOVE_LEFT, 1, 0), (MOVE_LEFT, MOVE_RIGHT, -1, 0), (MOVE_DOWN, MOVE_UP, 0, 1), (MOVE_UP, MOVE_DOWN, 0, -1)]
        .into_iter()
        .find(|&(_, _, dx, dy)| sim.level.tile(sx + dx, sy + dy) == FLOOR)
        .map(|(off, back, _, _)| (off, back))
        .expect("a floor tile next to the start");
    sim.step(off);
    let ev = sim.step(MOVE_SURFACE);
    assert!(ev.applied && !sim.is_over() && !ev.events.contains(&Event::Surfaced), "off the way up it is a wait");
    sim.step(back);
    assert_eq!((sim.x, sim.y), (sx, sy));
    let ev = sim.step(MOVE_SURFACE);
    assert!(ev.events.contains(&Event::Surfaced), "{ev:?}");
    assert!(sim.is_over() && sim.view().returned && sim.view().over);
    assert!(!sim.step(MOVE_LEFT).applied, "moves after the return are ignored");
    let out = sim.outcome();
    assert!(out.returned && !out.died && !out.quit, "{out:?}");
    assert!(out.death_pos.is_none() && out.dropped_into_grave.is_empty() && out.burned.is_empty());
    assert_eq!(out.moves_applied, 4);
}

/// Tiles reachable from a level's start without entering `blocked` (stairs count, are not crossed).
fn reachable_without(l: &Level, blocked: &[(i32, i32)]) -> std::vec::Vec<bool> {
    let mut seen = vec![false; (W * H) as usize];
    let mut queue = vec![l.start];
    seen[(l.start.1 * W + l.start.0) as usize] = true;
    while let Some((x, y)) = queue.pop() {
        if l.tile(x, y) == STAIRS {
            continue;
        }
        for (dx, dy) in [(0, 1), (0, -1), (1, 0), (-1, 0)] {
            let (nx, ny) = (x + dx, y + dy);
            if l.passable(nx, ny) && !blocked.contains(&(nx, ny)) && !seen[(ny * W + nx) as usize] {
                seen[(ny * W + nx) as usize] = true;
                queue.push((nx, ny));
            }
        }
    }
    seen
}

/// The gas benchmark's grave spots never cut a map apart: with all of them taken every other tile
/// reachable from the start still is (the stairs too), and no two spots touch.
#[test]
fn safe_grave_spots_never_cut_the_map() {
    for seed in 0..200u64 {
        for depth in 1..=10u32 {
            let l = generate_level(seed, depth, &[]);
            let spots = safe_grave_spots(seed, depth, rules::GRAVES_PER_DEPTH as usize);
            assert_eq!(spots.len(), rules::GRAVES_PER_DEPTH as usize, "seed {seed} depth {depth}");
            let blocked: std::vec::Vec<(i32, i32)> = spots.iter().map(|&(x, y)| (i32::from(x), i32::from(y))).collect();
            let before = reachable_without(&l, &[]);
            let after = reachable_without(&l, &blocked);
            for i in 0..(W * H) as usize {
                let cell = (i as i32 % W, i as i32 / W);
                if blocked.contains(&cell) {
                    assert!(before[i] && l.tile(cell.0, cell.1) == FLOOR && cell != l.start, "seed {seed} depth {depth} {cell:?}");
                } else {
                    assert_eq!(before[i], after[i], "seed {seed} depth {depth}: {cell:?} cut off");
                }
            }
            for (a, &p) in blocked.iter().enumerate() {
                assert!(blocked[a + 1..].iter().all(|&q| (p.0 - q.0).abs() > 1 || (p.1 - q.1).abs() > 1), "seed {seed} depth {depth}: spots touch");
            }
        }
    }
}

/// The filling bot of the gas benchmark: for every spot of depths 10 down to 1, with the graves
/// placed so far in the entry, a log ends on the spot (a quit there, or a death on it), so the
/// grave lands exactly where planned, and no grave is looted on the way.
#[test]
fn filling_bot_ends_its_log_on_each_grave_spot() {
    let mut retries = 0;
    for seed in [3u64, 77, 1234, 0xDEAD] {
        let mut graves: std::vec::Vec<Grave> = vec![];
        for depth in (1..=10u32).rev() {
            for spot in safe_grave_spots(seed, depth, rules::GRAVES_PER_DEPTH as usize) {
                let entry = DiveEntry { start_depth: 1, inventory: vec![(ITEM_GOLD, 1)], graves: graves.clone() };
                let (delay, moves) = (0..40)
                    .find_map(|delay| moves_to_quit_at(seed, &entry, depth, spot, MAX_MOVES as usize, delay).map(|m| (delay, m)))
                    .unwrap_or_else(|| panic!("seed {seed}: no log reaches {spot:?} at depth {depth}"));
                retries += delay;
                let out = run_dive(seed, &entry, &moves);
                assert!(out.died && !out.returned, "seed {seed} depth {depth} {spot:?}: {out:?}");
                assert_eq!((out.final_depth, out.death_pos), (depth, Some(spot)), "seed {seed}");
                assert!(out.looted_graves.is_empty(), "seed {seed} depth {depth}: looted {:?}", out.looted_graves);
                graves.push(Grave { id: graves.len() as u32, depth, x: spot.0, y: spot.1, items: out.dropped_into_grave });
            }
        }
        assert_eq!(graves.len(), 10 * rules::GRAVES_PER_DEPTH as usize);
    }
    std::println!("opening waits needed in total: {retries}");
}
