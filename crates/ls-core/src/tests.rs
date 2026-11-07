extern crate std;

use super::bot::{generate_moves, Style};
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
    assert_eq!(a.moves_applied, 1000);
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
        assert!(!out.died, "seed {seed} died at depth {}", out.final_depth);
        assert_eq!(out.moves_applied, 3000);
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
        if !out.died {
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
            depth: 1 + rng.below(3),
            x: rng.below(32) as u8,
            y: rng.below(32) as u8,
            items: vec![(ITEM_GOLD, 1 + rng.below(9)), (ITEM_BLADE, rng.below(2))],
        })
        .collect();
    let entry = DiveEntry {
        start_depth: 1 + rng.below(3),
        inventory: vec![(ITEM_GOLD, rng.below(50)), (ITEM_BLADE, rng.below(3)), (ITEM_PLATE, rng.below(2))],
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
    assert!(!sim.view().over);
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
