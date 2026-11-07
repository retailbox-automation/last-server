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

    let grave = Grave { depth: dead.final_depth, x: gx, y: gy, items: dead.dropped_into_grave };
    let bob_entry = DiveEntry { start_depth: 1, inventory: vec![], graves: vec![grave] };
    let looter = generate_moves(seed, &bob_entry, 400, Style::Looter);
    let bob = run_dive(seed, &bob_entry, &looter);
    assert_eq!(bob.looted_graves, vec![0]);
}

#[test]
fn grave_on_a_wall_snaps_to_floor() {
    let base = generate_level(5, 1, &[]);
    // (0, 0) is always a wall.
    let g = Grave { depth: 1, x: 0, y: 0, items: vec![(ITEM_GOLD, 1)] };
    let l = generate_level(5, 1, &[g]);
    assert_eq!(l.graves.len(), 1);
    assert_ne!(base.tile(l.graves[0].x, l.graves[0].y), WALL);
}

#[test]
fn walls_are_no_ops_that_count() {
    let entry = DiveEntry::default();
    let mut dive = Dive::new(9, &entry);
    dive.level.monsters.clear();
    let mut xs = vec![];
    for _ in 0..40 {
        assert!(dive.step(MOVE_LEFT));
        xs.push(dive.x);
    }
    // The player walks left until a wall, then stays put while the moves keep counting.
    let last = *xs.last().unwrap();
    assert!(xs.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] - 1));
    assert_eq!(dive.level.tile(last - 1, dive.y), WALL);
    assert_eq!(dive.finish().moves_applied, 40);
}

#[test]
fn item_names_round_trip() {
    for id in ITEM_IDS {
        assert_eq!(item_id(item_name(id).unwrap()), Some(id));
    }
    assert_eq!(item_name(0), None);
    assert_eq!(item_id("sword"), None);
}
