//! Last Server world contract (core v1).
//!
//! A dive is two calls. `start_dive(loadout)` locks what the player chooses to carry and hands
//! out a ticket with the day's seed (the contract's, not the player's). `settle_dive(dive_id,
//! moves)` re-runs `ls_core` on that seed and the move log, then applies the outcome: finds are
//! minted, looted graves move to the player, used-up and broken items burn, and on death what the
//! player carried becomes a grave at that depth for anyone to loot. A dive left open past its day
//! can be expired by anyone once `rules::EXPIRE_GRACE_SECONDS` have passed since its start: like
//! a quit, its loadout pays the death toll and the rest becomes a grave at the start depth.
//!
//! Graves are stored per depth (`LookupMap<depth, Vec<GraveRecord>>`, at most
//! `rules::GRAVES_PER_DEPTH` each, the oldest crumbles), and `settle_dive` reads only the depths
//! the dive enters, so its gas does not grow with the number of graves on the server.
//!
//! Items are NEP-245 multi-tokens with a hand-written subset: `mt_transfer`, `mt_batch_transfer`,
//! `mt_balance_of`, `mt_batch_balance_of`, `mt_supply`, `mt_batch_supply`, `mt_token` and the
//! `mt_mint` / `mt_transfer` / `mt_burn` events. Not implemented: `mt_transfer_call` /
//! `mt_resolve_transfer`, approvals, metadata, NEP-145 storage deposits. Locked (carried) items
//! cannot be transferred until the dive is settled or expired. Without storage deposits the
//! contract pays for every balance entry, so items go only to accounts that have started a dive.

use ls_core::rules::{DEATH_TOLL_GOLD_DIV, EXPIRE_GRACE_SECONDS, GRAVES_PER_DEPTH};
use ls_core::{DiveEntry, DiveOutcome, Grave, GraveSource, ItemId, MAX_MOVES, START_DEPTH};
use near_sdk::json_types::{Base64VecU8, U128, U64};
use near_sdk::serde::Serialize;
use near_sdk::store::{IterableMap, LookupMap};
use near_sdk::{env, near, require, AccountId, BorshStorageKey, PanicOnDefault};

/// Length of a game day. `current_day = block_timestamp / DAY_SECONDS`.
pub const DAY_SECONDS: u64 = 86_400;
const NS_PER_SECOND: u64 = 1_000_000_000;

#[derive(BorshStorageKey)]
#[near]
enum StorageKey {
    Balances,
    Locked,
    Supply,
    Graves,
    GraveCounts,
    Active,
    Players,
    Economy,
    Stats,
}

/// A grave on the server: what a dead player left at a depth.
#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct GraveRecord {
    pub id: u32,
    pub owner_id: AccountId,
    pub depth: u32,
    pub x: u8,
    pub y: u8,
    pub items: Vec<(ItemId, u32)>,
    /// Day the grave was made and the dive that made it.
    pub day: u32,
    pub dive_id: u64,
}

/// A dive between `start_dive` and `settle_dive` / `expire_dive`.
#[near(serializers = [borsh])]
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveDive {
    pub dive_id: u64,
    /// Block timestamp (ns) of `start_dive`.
    pub started_at: u64,
    pub day: u32,
    pub seed: u64,
    pub start_depth: u32,
    pub loadout: Vec<(ItemId, u32)>,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug, Default)]
pub struct PlayerStats {
    pub dives: u32,
    pub deaths: u32,
    pub deepest: u32,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug, Default)]
pub struct Economy {
    pub minted: u128,
    pub burned: u128,
    pub to_graves: u128,
    pub from_graves: u128,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug, Default)]
pub struct DayStats {
    pub dives: u32,
    pub deaths: u32,
    pub graves_looted: u32,
    pub moves: u64,
}

/// `start_dive` result and `active_dive` view.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq)]
pub struct DiveTicket {
    pub dive_id: U64,
    pub day: u32,
    pub seed: U64,
    pub start_depth: u32,
}

/// `graves` view row.
#[near(serializers = [json])]
pub struct GraveView {
    pub id: u32,
    pub owner_id: AccountId,
    pub depth: u32,
    pub x: u8,
    pub y: u8,
    pub items: Vec<(String, U128)>,
    pub day: u32,
    pub dive_id: U64,
}

#[near(serializers = [json])]
pub struct PlayerView {
    /// Balances free to carry or transfer.
    pub stash: Vec<(String, U128)>,
    /// Balances carried in the active dive (not transferable).
    pub locked: Vec<(String, U128)>,
    pub dives: u32,
    pub deaths: u32,
    pub deepest: u32,
}

#[near(serializers = [json])]
pub struct EconomyView {
    pub minted: U128,
    pub burned: U128,
    pub to_graves: U128,
    pub from_graves: U128,
}

#[near(serializers = [json])]
pub struct DayStatsView {
    pub dives: u32,
    pub deaths: u32,
    pub graves_looted: u32,
    pub moves: U64,
}

#[near(serializers = [json])]
pub struct CoreVersion {
    pub contract: String,
    pub ls_core: String,
    /// sha256 of crates/ls-core/src/rules.rs (all the numbers).
    pub rules_sha256: String,
    /// sha256 of crates/ls-core/src/lib.rs followed by rules.rs.
    pub core_sha256: String,
}

/// NEP-245 `Token`.
#[near(serializers = [json])]
pub struct Token {
    pub token_id: String,
    pub owner_id: Option<AccountId>,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Contract {
    /// Mixed into every day seed; set from the block's random seed at init.
    salt: Vec<u8>,
    /// (token_id, owner) -> balance, locked included.
    balances: LookupMap<(String, AccountId), u128>,
    /// (token_id, owner) -> amount carried in the active dive.
    locked: LookupMap<(String, AccountId), u128>,
    supply: LookupMap<String, u128>,
    token_ids: Vec<String>,
    /// depth -> graves, oldest first.
    graves: LookupMap<u32, Vec<GraveRecord>>,
    /// depth -> number of graves (only depths with graves).
    grave_counts: IterableMap<u32, u32>,
    next_grave_id: u32,
    next_dive_id: u64,
    active: LookupMap<AccountId, ActiveDive>,
    players: LookupMap<AccountId, PlayerStats>,
    economy: LookupMap<String, Economy>,
    stats: LookupMap<u32, DayStats>,
}

#[near]
impl Contract {
    #[init]
    pub fn new() -> Self {
        let token_ids = ls_core::ITEM_IDS.iter().map(|&id| token_name(id).to_string()).collect();
        Self {
            salt: env::random_seed(),
            balances: LookupMap::new(StorageKey::Balances),
            locked: LookupMap::new(StorageKey::Locked),
            supply: LookupMap::new(StorageKey::Supply),
            token_ids,
            graves: LookupMap::new(StorageKey::Graves),
            grave_counts: IterableMap::new(StorageKey::GraveCounts),
            next_grave_id: 0,
            next_dive_id: 0,
            active: LookupMap::new(StorageKey::Active),
            players: LookupMap::new(StorageKey::Players),
            economy: LookupMap::new(StorageKey::Economy),
            stats: LookupMap::new(StorageKey::Stats),
        }
    }

    // ---------------------------------------------------------------------------------------
    // Game
    // ---------------------------------------------------------------------------------------

    /// Starts a dive carrying `loadout` (`[[token_id, amount], ...]`, from the caller's stash).
    /// The items stay the caller's NEP-245 balance but are locked until the dive ends.
    pub fn start_dive(&mut self, loadout: Vec<(String, U128)>) -> DiveTicket {
        let player = env::predecessor_account_id();
        require!(self.active.get(&player).is_none(), "an active dive already exists: settle or expire it first");
        let mut items: Vec<(ItemId, u32)> = Vec::new();
        for (token, amount) in &loadout {
            let id = ls_core::item_id(token).unwrap_or_else(|| env::panic_str("unknown token_id"));
            require!(amount.0 > 0, "amount must be positive");
            require!(items.iter().all(|e| e.0 != id), "duplicate token_id in loadout");
            let n = u32::try_from(amount.0).unwrap_or_else(|_| env::panic_str("amount too large"));
            require!(self.balance(token, &player) >= amount.0, "insufficient balance for loadout");
            items.push((id, n));
        }
        for &(id, n) in &items {
            // Nothing is locked without an active dive, so this is a fresh entry.
            self.locked.insert((token_name(id).to_string(), player.clone()), u128::from(n));
        }
        let day = self.current_day();
        let dive = ActiveDive {
            dive_id: self.next_dive_id,
            started_at: env::block_timestamp(),
            day,
            seed: self.seed_of(day),
            start_depth: START_DEPTH,
            loadout: items,
        };
        self.next_dive_id += 1;
        let ticket = ticket_of(&dive);
        self.player_mut(&player).dives += 1;
        self.day_mut(day).dives += 1;
        emit_game(
            "dive_started",
            &DiveStartedLog {
                account_id: &player,
                dive_id: ticket.dive_id,
                day,
                seed: ticket.seed,
                start_depth: dive.start_depth,
                loadout: named(&dive.loadout),
            },
        );
        self.active.insert(player, dive);
        ticket
    }

    /// Ends the caller's active dive: re-runs the rules on the ticket's seed and `moves`
    /// (base64, one byte per move, at most `MAX_MOVES`) and applies the outcome.
    pub fn settle_dive(&mut self, dive_id: U64, moves: Base64VecU8) -> DiveOutcome {
        let player = env::predecessor_account_id();
        let dive = self.active.get(&player).cloned().unwrap_or_else(|| env::panic_str("no active dive for this account"));
        require!(dive.dive_id == dive_id.0, "dive_id is not this account's active dive");
        require!(moves.0.len() <= MAX_MOVES as usize, "move log longer than MAX_MOVES (4000)");

        let entry = DiveEntry { start_depth: dive.start_depth, inventory: dive.loadout.clone(), graves: Vec::new() };
        let outcome = {
            let mut src = StateGraves { graves: &self.graves };
            ls_core::run_dive_with(dive.seed, &entry, &moves.0, &mut src)
        };
        self.active.remove(&player);
        self.unlock(&player, &dive.loadout);
        let me = env::current_account_id();

        // 1. New items from the dungeon are minted to the player.
        if !outcome.picked.is_empty() {
            let items = named_u128(&outcome.picked);
            for (token, amount) in &items {
                self.mint(token, &player, *amount);
            }
            emit_mint(&player, &items, Some("dive finds"));
        }

        // 2. Looted graves move from the contract to the player.
        if !outcome.looted_graves.is_empty() {
            let mut loot: Vec<(ItemId, u32)> = Vec::new();
            let mut rows = Vec::new();
            for &id in &outcome.looted_graves {
                let grave = self.take_grave(id, dive.start_depth, outcome.final_depth);
                for &(item, n) in &grave.items {
                    ls_core::add_item(&mut loot, item, n);
                }
                rows.push(LootedRow { grave_id: grave.id, owner_id: grave.owner_id.clone(), depth: grave.depth });
            }
            let items = named_u128(&loot);
            for (token, amount) in &items {
                self.economy_mut(token).from_graves += amount;
            }
            self.move_tokens(&me, &player, &items);
            emit_transfer(&me, &player, &items, Some("grave loot"));
            emit_game("grave_looted", &GraveLootedLog { looter_id: &player, dive_id, graves: rows });
            self.day_mut(dive.day).graves_looted += outcome.looted_graves.len() as u32;
        }

        // 3. Used up, broken and (on death) the toll: burned from the player.
        if !outcome.burned.is_empty() {
            let items = named_u128(&outcome.burned);
            for (token, amount) in &items {
                self.burn(token, &player, *amount);
            }
            emit_burn(&player, &items, Some(if outcome.died { "used up, broken, death toll" } else { "used up, broken" }));
        }

        // 4. Death: what was left goes into a new grave held by the contract.
        let mut grave_id = None;
        if outcome.died {
            if !outcome.dropped_into_grave.is_empty() {
                let items = named_u128(&outcome.dropped_into_grave);
                for (token, amount) in &items {
                    self.economy_mut(token).to_graves += amount;
                }
                self.move_tokens(&player, &me, &items);
                emit_transfer(&player, &me, &items, Some("grave"));
                let (x, y) = outcome.death_pos.unwrap_or_else(|| env::panic_str("death without a position"));
                let day = self.current_day();
                grave_id = Some(self.add_grave(
                    &player,
                    outcome.final_depth,
                    x,
                    y,
                    outcome.dropped_into_grave.clone(),
                    day,
                    dive.dive_id,
                ));
            }
            self.player_mut(&player).deaths += 1;
            self.day_mut(dive.day).deaths += 1;
        }

        // Every log above and below is in `tests::worst_settle_logs`, which checks NEAR's limits on
        // the logs of one call; a new log here goes there too.
        let stats = self.player_mut(&player);
        stats.deepest = stats.deepest.max(outcome.final_depth);
        self.day_mut(dive.day).moves += u64::from(outcome.moves_applied);
        emit_game(
            "dive_settled",
            &DiveSettledLog {
                account_id: &player,
                dive_id,
                day: dive.day,
                died: outcome.died,
                final_depth: outcome.final_depth,
                moves_applied: outcome.moves_applied,
                kills: outcome.kills,
                picked: named(&outcome.picked),
                burned: named(&outcome.burned),
                looted_graves: outcome.looted_graves.clone(),
                grave_id,
                vault_opened: outcome.vault_opened,
            },
        );
        outcome
    }

    /// Anyone may end a dive left open past its day, once `EXPIRE_GRACE_SECONDS` have passed
    /// since it started: it counts as a death, and the loadout becomes a grave at the start depth
    /// (position from the seed), minus the death toll, as for a quit.
    pub fn expire_dive(&mut self, account_id: AccountId) {
        let dive = self.active.get(&account_id).cloned().unwrap_or_else(|| env::panic_str("no active dive for this account"));
        if let Some(why) = expire_refusal(&dive, env::block_timestamp()) {
            env::panic_str(&why);
        }
        let today = self.current_day();
        self.active.remove(&account_id);
        self.unlock(&account_id, &dive.loadout);
        let mut grave_id = None;
        let (grave, toll) = expire_split(&dive.loadout);
        if !toll.is_empty() {
            let items = named_u128(&toll);
            for (token, amount) in &items {
                self.burn(token, &account_id, *amount);
            }
            emit_burn(&account_id, &items, Some("death toll"));
        }
        if !grave.is_empty() {
            let me = env::current_account_id();
            let items = named_u128(&grave);
            for (token, amount) in &items {
                self.economy_mut(token).to_graves += amount;
            }
            self.move_tokens(&account_id, &me, &items);
            emit_transfer(&account_id, &me, &items, Some("expired dive"));
            let (x, y) = ls_core::expired_grave_spot(dive.seed, dive.dive_id);
            grave_id = Some(self.add_grave(&account_id, dive.start_depth, x, y, grave, today, dive.dive_id));
        }
        self.player_mut(&account_id).deaths += 1;
        self.day_mut(dive.day).deaths += 1;
        emit_game(
            "dive_expired",
            &DiveExpiredLog {
                account_id: &account_id,
                dive_id: U64(dive.dive_id),
                day: dive.day,
                expired_by: env::predecessor_account_id(),
                grave_id,
            },
        );
    }

    // ---------------------------------------------------------------------------------------
    // Views
    // ---------------------------------------------------------------------------------------

    pub fn current_day(&self) -> u32 {
        day_of(env::block_timestamp())
    }

    /// The seed of every dive started on `day`: u64 (little-endian) of sha256(day LE ‖ salt).
    pub fn day_seed(&self, day: u32) -> U64 {
        U64(self.seed_of(day))
    }

    /// Exactly what `settle_dive` feeds into the rules for this account's active dive (the
    /// loadout and the graves on every depth from the start depth down, in stored order), so a
    /// client can simulate it with `DiveSim::new(ticket.seed, &entry)`. Without an active dive:
    /// the start depth, an empty inventory and the same graves.
    pub fn dive_entry(&self, account_id: AccountId) -> DiveEntry {
        let (start_depth, inventory) = match self.active.get(&account_id) {
            Some(d) => (d.start_depth, d.loadout.clone()),
            None => (START_DEPTH, Vec::new()),
        };
        let mut depths: Vec<u32> = self.grave_counts.keys().copied().filter(|&d| d >= start_depth).collect();
        depths.sort_unstable();
        let mut graves = Vec::new();
        for d in depths {
            if let Some(list) = self.graves.get(&d) {
                graves.extend(list.iter().map(core_grave));
            }
        }
        DiveEntry { start_depth, inventory, graves }
    }

    pub fn active_dive(&self, account_id: AccountId) -> Option<DiveTicket> {
        self.active.get(&account_id).map(ticket_of)
    }

    pub fn graves(&self, depth: u32) -> Vec<GraveView> {
        self.graves.get(&depth).map(|list| list.iter().map(grave_view).collect()).unwrap_or_default()
    }

    /// `[[depth, number of graves], ...]` for every depth with graves, shallowest first.
    pub fn graves_summary(&self) -> Vec<(u32, u32)> {
        let mut rows: Vec<(u32, u32)> = self.grave_counts.iter().map(|(&d, &n)| (d, n)).collect();
        rows.sort_unstable();
        rows
    }

    pub fn player(&self, account_id: AccountId) -> PlayerView {
        let mut stash = Vec::new();
        let mut locked = Vec::new();
        for t in &self.token_ids {
            let total = self.balance(t, &account_id);
            let l = self.locked_of(t, &account_id);
            if total > l {
                stash.push((t.clone(), U128(total - l)));
            }
            if l > 0 {
                locked.push((t.clone(), U128(l)));
            }
        }
        let s = self.players.get(&account_id).cloned().unwrap_or_default();
        PlayerView { stash, locked, dives: s.dives, deaths: s.deaths, deepest: s.deepest }
    }

    pub fn economy(&self, token_id: String) -> EconomyView {
        require!(self.known(&token_id), "unknown token_id");
        let e = self.economy.get(&token_id).cloned().unwrap_or_default();
        EconomyView {
            minted: U128(e.minted),
            burned: U128(e.burned),
            to_graves: U128(e.to_graves),
            from_graves: U128(e.from_graves),
        }
    }

    pub fn stats_by_day(&self, day: u32) -> DayStatsView {
        let s = self.stats.get(&day).cloned().unwrap_or_default();
        DayStatsView { dives: s.dives, deaths: s.deaths, graves_looted: s.graves_looted, moves: U64(s.moves) }
    }

    pub fn core_version(&self) -> CoreVersion {
        CoreVersion {
            contract: env!("CARGO_PKG_VERSION").to_string(),
            ls_core: ls_core::VERSION.to_string(),
            rules_sha256: ls_core::RULES_HASH.to_string(),
            core_sha256: ls_core::CORE_HASH.to_string(),
        }
    }

    // ---------------------------------------------------------------------------------------
    // NEP-245 subset
    // ---------------------------------------------------------------------------------------

    #[payable]
    pub fn mt_transfer(
        &mut self,
        receiver_id: AccountId,
        token_id: String,
        amount: U128,
        approval: Option<(AccountId, u64)>,
        memo: Option<String>,
    ) {
        near_sdk::assert_one_yocto();
        require!(approval.is_none(), "approvals are not supported");
        let sender = env::predecessor_account_id();
        self.checked_transfer(&sender, &receiver_id, vec![(token_id, amount.0)], memo.as_deref());
    }

    #[payable]
    pub fn mt_batch_transfer(
        &mut self,
        receiver_id: AccountId,
        token_ids: Vec<String>,
        amounts: Vec<U128>,
        approvals: Option<Vec<Option<(AccountId, u64)>>>,
        memo: Option<String>,
    ) {
        near_sdk::assert_one_yocto();
        require!(token_ids.len() == amounts.len(), "token_ids and amounts differ in length");
        if let Some(approvals) = &approvals {
            require!(approvals.len() == token_ids.len(), "approvals and token_ids differ in length");
            require!(approvals.iter().all(Option::is_none), "approvals are not supported");
        }
        let sender = env::predecessor_account_id();
        let items = token_ids.into_iter().zip(amounts.into_iter().map(|a| a.0)).collect();
        self.checked_transfer(&sender, &receiver_id, items, memo.as_deref());
    }

    pub fn mt_balance_of(&self, account_id: AccountId, token_id: String) -> U128 {
        U128(self.balance(&token_id, &account_id))
    }

    pub fn mt_batch_balance_of(&self, account_id: AccountId, token_ids: Vec<String>) -> Vec<U128> {
        token_ids.iter().map(|t| U128(self.balance(t, &account_id))).collect()
    }

    pub fn mt_supply(&self, token_id: String) -> Option<U128> {
        self.known(&token_id).then(|| U128(self.supply.get(&token_id).copied().unwrap_or(0)))
    }

    pub fn mt_batch_supply(&self, token_ids: Vec<String>) -> Vec<Option<U128>> {
        token_ids.into_iter().map(|t| self.mt_supply(t)).collect()
    }

    pub fn mt_token(&self, token_ids: Vec<String>) -> Vec<Option<Token>> {
        token_ids
            .into_iter()
            .map(|t| self.known(&t).then(|| Token { token_id: t, owner_id: None }))
            .collect()
    }
}

/// Graves of a depth straight from contract state, read when the dive enters that depth.
struct StateGraves<'a> {
    graves: &'a LookupMap<u32, Vec<GraveRecord>>,
}

impl GraveSource for StateGraves<'_> {
    fn graves_at(&mut self, depth: u32) -> Vec<Grave> {
        self.graves.get(&depth).map(|list| list.iter().map(core_grave).collect()).unwrap_or_default()
    }
}

impl Contract {
    fn seed_of(&self, day: u32) -> u64 {
        let mut input = day.to_le_bytes().to_vec();
        input.extend_from_slice(&self.salt);
        let h = env::sha256(&input);
        u64::from_le_bytes(h[..8].try_into().expect("sha256 is 32 bytes"))
    }

    fn known(&self, token_id: &str) -> bool {
        self.token_ids.iter().any(|t| t == token_id)
    }

    fn balance(&self, token_id: &str, owner: &AccountId) -> u128 {
        self.balances.get(&(token_id.to_string(), owner.clone())).copied().unwrap_or(0)
    }

    fn locked_of(&self, token_id: &str, owner: &AccountId) -> u128 {
        self.locked.get(&(token_id.to_string(), owner.clone())).copied().unwrap_or(0)
    }

    fn add_balance(&mut self, token_id: &str, owner: &AccountId, amount: u128) {
        let key = (token_id.to_string(), owner.clone());
        let b = self.balances.get(&key).copied().unwrap_or(0);
        self.balances.insert(key, b.checked_add(amount).unwrap_or_else(|| env::panic_str("balance overflow")));
    }

    fn sub_balance(&mut self, token_id: &str, owner: &AccountId, amount: u128) {
        let key = (token_id.to_string(), owner.clone());
        let b = self.balances.get(&key).copied().unwrap_or(0);
        require!(b >= amount, "insufficient balance");
        if b == amount {
            self.balances.remove(&key);
        } else {
            self.balances.insert(key, b - amount);
        }
    }

    fn mint(&mut self, token_id: &str, owner: &AccountId, amount: u128) {
        self.add_balance(token_id, owner, amount);
        let s = self.supply.get(token_id).copied().unwrap_or(0);
        self.supply.insert(token_id.to_string(), s.checked_add(amount).unwrap_or_else(|| env::panic_str("supply overflow")));
        self.economy_mut(token_id).minted += amount;
    }

    fn burn(&mut self, token_id: &str, owner: &AccountId, amount: u128) {
        self.sub_balance(token_id, owner, amount);
        let s = self.supply.get(token_id).copied().unwrap_or(0);
        self.supply.insert(token_id.to_string(), s.checked_sub(amount).unwrap_or_else(|| env::panic_str("supply underflow")));
        self.economy_mut(token_id).burned += amount;
    }

    fn move_tokens(&mut self, from: &AccountId, to: &AccountId, items: &[(String, u128)]) {
        for (token, amount) in items {
            self.sub_balance(token, from, *amount);
            self.add_balance(token, to, *amount);
        }
    }

    fn unlock(&mut self, owner: &AccountId, loadout: &[(ItemId, u32)]) {
        for &(id, _) in loadout {
            self.locked.remove(&(token_name(id).to_string(), owner.clone()));
        }
    }

    fn economy_mut(&mut self, token_id: &str) -> &mut Economy {
        self.economy.entry(token_id.to_string()).or_default()
    }

    fn player_mut(&mut self, account: &AccountId) -> &mut PlayerStats {
        self.players.entry(account.clone()).or_default()
    }

    fn day_mut(&mut self, day: u32) -> &mut DayStats {
        self.stats.entry(day).or_default()
    }

    /// User-initiated transfer: validates every item against the sender's unlocked balance,
    /// moves balances, emits `mt_transfer`.
    fn checked_transfer(&mut self, sender: &AccountId, receiver: &AccountId, items: Vec<(String, u128)>, memo: Option<&str>) {
        require!(sender != receiver, "sender and receiver must differ");
        require!(receiver != &env::current_account_id(), "the world contract does not take transfers");
        // The contract pays the storage of every balance entry: items go only to accounts that have
        // started a dive (at most one entry per item kind each), never to made-up account ids.
        require!(self.players.contains_key(receiver), "the receiver has never started a dive: items go only to players");
        require!(!items.is_empty(), "nothing to transfer");
        for (token, amount) in &items {
            require!(self.known(token), "unknown token_id");
            require!(*amount > 0, "amount must be positive");
            let balance = self.balance(token, sender);
            let free = balance - self.locked_of(token, sender).min(balance);
            if *amount > free {
                require!(*amount > balance, "items carried in an active dive cannot be transferred");
                env::panic_str("insufficient balance");
            }
            self.sub_balance(token, sender, *amount);
            self.add_balance(token, receiver, *amount);
        }
        emit_transfer(sender, receiver, &items, memo);
    }

    /// Removes a grave the dive looted. It was read from one of the depths the dive entered.
    fn take_grave(&mut self, id: u32, from_depth: u32, to_depth: u32) -> GraveRecord {
        for depth in from_depth.max(1)..=to_depth {
            let Some(list) = self.graves.get_mut(&depth) else { continue };
            let Some(i) = list.iter().position(|g| g.id == id) else { continue };
            let grave = list.remove(i);
            let left = list.len() as u32;
            if left == 0 {
                self.graves.remove(&depth);
                self.grave_counts.remove(&depth);
            } else {
                self.grave_counts.insert(depth, left);
            }
            return grave;
        }
        env::panic_str("looted grave is missing")
    }

    /// Stores a new grave (its items are already held by the contract). Beyond
    /// `GRAVES_PER_DEPTH` at that depth the oldest grave crumbles: its items are burned.
    #[allow(clippy::too_many_arguments)]
    fn add_grave(&mut self, owner: &AccountId, depth: u32, x: u8, y: u8, items: Vec<(ItemId, u32)>, day: u32, dive_id: u64) -> u32 {
        let id = self.next_grave_id;
        self.next_grave_id = id.checked_add(1).unwrap_or_else(|| env::panic_str("grave ids exhausted"));
        let grave = GraveRecord { id, owner_id: owner.clone(), depth, x, y, items, day, dive_id };
        let created = GraveLog::of(&grave);
        let list = self.graves.entry(depth).or_default();
        let crumbled = if list.len() as u32 >= GRAVES_PER_DEPTH { Some(list.remove(0)) } else { None };
        list.push(grave);
        let count = list.len() as u32;
        self.grave_counts.insert(depth, count);
        if let Some(old) = crumbled {
            let me = env::current_account_id();
            let items = named_u128(&old.items);
            for (token, amount) in &items {
                self.burn(token, &me, *amount);
            }
            emit_burn(&me, &items, Some("grave crumbled"));
            emit_game("grave_crumbled", &GraveLog::of(&old));
        }
        emit_game("grave_created", &created);
        id
    }
}

fn token_name(id: ItemId) -> &'static str {
    ls_core::item_name(id).unwrap_or_else(|| env::panic_str("unknown item id"))
}

fn named(items: &[(ItemId, u32)]) -> Vec<(String, U128)> {
    items.iter().map(|&(id, n)| (token_name(id).to_string(), U128(u128::from(n)))).collect()
}

fn named_u128(items: &[(ItemId, u32)]) -> Vec<(String, u128)> {
    items.iter().map(|&(id, n)| (token_name(id).to_string(), u128::from(n))).collect()
}

fn core_grave(g: &GraveRecord) -> Grave {
    Grave { id: g.id, depth: g.depth, x: g.x, y: g.y, items: g.items.clone() }
}

fn grave_view(g: &GraveRecord) -> GraveView {
    GraveView {
        id: g.id,
        owner_id: g.owner_id.clone(),
        depth: g.depth,
        x: g.x,
        y: g.y,
        items: named(&g.items),
        day: g.day,
        dive_id: U64(g.dive_id),
    }
}

/// The game day of a block timestamp (ns).
fn day_of(timestamp_ns: u64) -> u32 {
    u32::try_from(timestamp_ns / NS_PER_SECOND / DAY_SECONDS).unwrap_or(u32::MAX)
}

/// Why `dive` cannot be expired at block time `now` (ns), or `None` when anyone may expire it:
/// its day is over and `EXPIRE_GRACE_SECONDS` have passed since it started (so a dive started just
/// before midnight is not expired while its owner is still playing).
fn expire_refusal(dive: &ActiveDive, now: u64) -> Option<String> {
    if dive.day >= day_of(now) {
        return Some("the dive's day is not over yet".to_string());
    }
    if now.saturating_sub(dive.started_at) / NS_PER_SECOND < EXPIRE_GRACE_SECONDS {
        return Some(format!("the dive started less than {EXPIRE_GRACE_SECONDS} seconds ago"));
    }
    None
}

/// What an expired dive's loadout becomes, `(grave, burned)`: the same as a quit in ls-core
/// (`DiveSim::outcome`), one gold in `DEATH_TOLL_GOLD_DIV` burned and the rest in the grave.
/// Without the toll, waiting for someone to expire the dive would be cheaper than quitting.
fn expire_split(loadout: &[(ItemId, u32)]) -> (Vec<(ItemId, u32)>, Vec<(ItemId, u32)>) {
    let (mut grave, mut toll) = (Vec::new(), Vec::new());
    for &(id, n) in loadout {
        let t = if id == ls_core::ITEM_GOLD { n / DEATH_TOLL_GOLD_DIV } else { 0 };
        ls_core::add_item(&mut toll, id, t);
        ls_core::add_item(&mut grave, id, n - t);
    }
    (grave, toll)
}

fn ticket_of(d: &ActiveDive) -> DiveTicket {
    DiveTicket { dive_id: U64(d.dive_id), day: d.day, seed: U64(d.seed), start_depth: d.start_depth }
}

// -------------------------------------------------------------------------------------------
// NEP-297 events: "nep245" 1.0.0 for tokens, "lastserver" 1.0.0 for the game
// -------------------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct Event<'a, T: Serialize> {
    standard: &'static str,
    version: &'static str,
    event: &'static str,
    data: &'a [T],
}

/// One NEP-297 log line.
fn event_line<T: Serialize>(standard: &'static str, event: &'static str, data: &[T]) -> String {
    let e = Event { standard, version: "1.0.0", event, data };
    let json = near_sdk::serde_json::to_string(&e).unwrap_or_else(|_| env::panic_str("event serializes"));
    format!("EVENT_JSON:{json}")
}

fn game_line<T: Serialize>(event: &'static str, row: &T) -> String {
    event_line("lastserver", event, core::slice::from_ref(row))
}

fn emit_game<T: Serialize>(event: &'static str, row: &T) {
    env::log_str(&game_line(event, row));
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct DiveStartedLog<'a> {
    account_id: &'a AccountId,
    dive_id: U64,
    day: u32,
    seed: U64,
    start_depth: u32,
    loadout: Vec<(String, U128)>,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct DiveSettledLog<'a> {
    account_id: &'a AccountId,
    dive_id: U64,
    day: u32,
    died: bool,
    final_depth: u32,
    moves_applied: u32,
    kills: u32,
    picked: Vec<(String, U128)>,
    burned: Vec<(String, U128)>,
    looted_graves: Vec<u32>,
    grave_id: Option<u32>,
    vault_opened: bool,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct DiveExpiredLog<'a> {
    account_id: &'a AccountId,
    dive_id: U64,
    day: u32,
    expired_by: AccountId,
    grave_id: Option<u32>,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct GraveLog {
    grave_id: u32,
    owner_id: AccountId,
    depth: u32,
    x: u8,
    y: u8,
    items: Vec<(String, U128)>,
}

impl GraveLog {
    fn of(g: &GraveRecord) -> Self {
        GraveLog { grave_id: g.id, owner_id: g.owner_id.clone(), depth: g.depth, x: g.x, y: g.y, items: named(&g.items) }
    }
}

/// One looted grave in `grave_looted`. Kept small on purpose: a dive loots up to
/// `rules::GRAVES_LOOTED_PER_DIVE` graves and all logs of a call share a 16 KB limit (items are in
/// the grave's `grave_created`).
#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct LootedRow {
    grave_id: u32,
    owner_id: AccountId,
    depth: u32,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct GraveLootedLog<'a> {
    looter_id: &'a AccountId,
    dive_id: U64,
    graves: Vec<LootedRow>,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct MtMintLog<'a> {
    owner_id: &'a AccountId,
    token_ids: Vec<&'a str>,
    amounts: Vec<U128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memo: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct MtBurnLog<'a> {
    owner_id: &'a AccountId,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorized_id: Option<&'a AccountId>,
    token_ids: Vec<&'a str>,
    amounts: Vec<U128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memo: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct MtTransferLog<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    authorized_id: Option<&'a AccountId>,
    old_owner_id: &'a AccountId,
    new_owner_id: &'a AccountId,
    token_ids: Vec<&'a str>,
    amounts: Vec<U128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memo: Option<&'a str>,
}

fn split(items: &[(String, u128)]) -> (Vec<&str>, Vec<U128>) {
    items.iter().map(|(t, n)| (t.as_str(), U128(*n))).unzip()
}

fn mint_line(owner: &AccountId, items: &[(String, u128)], memo: Option<&str>) -> String {
    let (token_ids, amounts) = split(items);
    event_line("nep245", "mt_mint", &[MtMintLog { owner_id: owner, token_ids, amounts, memo }])
}

fn burn_line(owner: &AccountId, items: &[(String, u128)], memo: Option<&str>) -> String {
    let (token_ids, amounts) = split(items);
    event_line("nep245", "mt_burn", &[MtBurnLog { owner_id: owner, authorized_id: None, token_ids, amounts, memo }])
}

fn transfer_line(from: &AccountId, to: &AccountId, items: &[(String, u128)], memo: Option<&str>) -> String {
    let (token_ids, amounts) = split(items);
    event_line(
        "nep245",
        "mt_transfer",
        &[MtTransferLog { authorized_id: None, old_owner_id: from, new_owner_id: to, token_ids, amounts, memo }],
    )
}

fn emit_mint(owner: &AccountId, items: &[(String, u128)], memo: Option<&str>) {
    env::log_str(&mint_line(owner, items, memo));
}

fn emit_burn(owner: &AccountId, items: &[(String, u128)], memo: Option<&str>) {
    env::log_str(&burn_line(owner, items, memo));
}

fn emit_transfer(from: &AccountId, to: &AccountId, items: &[(String, u128)], memo: Option<&str>) {
    env::log_str(&transfer_line(from, to, items, memo));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_core::rules::GRAVES_LOOTED_PER_DIVE;

    /// NEAR's limits on the logs of one function call (`limit_config.max_number_logs` and
    /// `max_total_log_length`, testnet protocol 88, read 2026-11-07).
    const MAX_NUMBER_LOGS: usize = 100;
    const MAX_TOTAL_LOG_LENGTH: usize = 16_384;

    /// Every log line one `settle_dive` can write, each at its longest, for a dive that loots
    /// `looted` graves and dies: finds minted, loot moved, looted graves listed, used-up items and
    /// the death toll burned, the rest moved into a new grave that makes the oldest grave at its
    /// depth crumble. Account ids have 64 characters (the protocol maximum), every number is its
    /// type's maximum and every item list holds all six item kinds.
    fn worst_settle_logs(looted: usize) -> Vec<String> {
        let id = |c: char| -> AccountId { c.to_string().repeat(64).parse().expect("valid account id") };
        let (player, world, owner) = (id('p'), id('w'), id('o'));
        let max_items: Vec<(ItemId, u32)> = ls_core::ITEM_IDS.iter().map(|&i| (i, u32::MAX)).collect();
        let items = named_u128(&max_items);
        let loot: Vec<(String, u128)> = items.iter().map(|(t, n)| (t.clone(), n * looted as u128)).collect();
        let grave = GraveRecord {
            id: u32::MAX,
            owner_id: owner.clone(),
            depth: u32::MAX,
            x: u8::MAX,
            y: u8::MAX,
            items: max_items.clone(),
            day: u32::MAX,
            dive_id: u64::MAX,
        };
        let rows = (0..looted).map(|_| LootedRow { grave_id: u32::MAX, owner_id: owner.clone(), depth: u32::MAX }).collect();
        vec![
            mint_line(&player, &items, Some("dive finds")),
            transfer_line(&world, &player, &loot, Some("grave loot")),
            game_line("grave_looted", &GraveLootedLog { looter_id: &player, dive_id: U64(u64::MAX), graves: rows }),
            burn_line(&player, &items, Some("used up, broken, death toll")),
            transfer_line(&player, &world, &items, Some("grave")),
            burn_line(&world, &items, Some("grave crumbled")),
            game_line("grave_crumbled", &GraveLog::of(&grave)),
            game_line("grave_created", &GraveLog::of(&grave)),
            game_line(
                "dive_settled",
                &DiveSettledLog {
                    account_id: &player,
                    dive_id: U64(u64::MAX),
                    day: u32::MAX,
                    died: true,
                    final_depth: u32::MAX,
                    moves_applied: u32::MAX,
                    kills: u32::MAX,
                    picked: named(&max_items),
                    burned: named(&max_items),
                    looted_graves: vec![u32::MAX; looted],
                    grave_id: Some(u32::MAX),
                    vault_opened: false,
                },
            ),
        ]
    }

    fn total(lines: &[String]) -> usize {
        lines.iter().map(String::len).sum()
    }

    #[test]
    fn worst_case_settle_logs_fit_near_limits() {
        let cap = GRAVES_LOOTED_PER_DIVE as usize;
        let lines = worst_settle_logs(cap);
        for line in &lines {
            let json = line.strip_prefix("EVENT_JSON:").expect("NEP-297 prefix");
            near_sdk::serde_json::from_str::<near_sdk::serde_json::Value>(json).expect("a log line is JSON");
        }
        let fixed = total(&worst_settle_logs(0));
        let per_grave = total(&worst_settle_logs(cap + 1)) - total(&lines);
        println!(
            "worst settle_dive logs at {cap} looted graves: {} lines, {} bytes of {MAX_TOTAL_LOG_LENGTH} (no graves: {fixed} bytes; {per_grave} bytes per grave; room for {} graves)",
            lines.len(),
            total(&lines),
            (MAX_TOTAL_LOG_LENGTH - fixed) / per_grave
        );
        for line in &lines {
            println!("  {} bytes  {}", line.len(), &line[..line.len().min(90)]);
        }
        assert!(lines.len() <= MAX_NUMBER_LOGS, "{} logs", lines.len());
        assert!(total(&lines) <= MAX_TOTAL_LOG_LENGTH, "{} bytes", total(&lines));
        // Control: the same model without the cap breaks the limit, as the review found for ~150.
        let uncapped = total(&worst_settle_logs(150));
        assert!(uncapped > MAX_TOTAL_LOG_LENGTH, "150 looted graves: {uncapped} bytes");
    }

    fn dive_started(day: u32, started_at: u64) -> ActiveDive {
        ActiveDive { dive_id: 0, started_at, day, seed: 0, start_depth: 1, loadout: vec![] }
    }

    #[test]
    fn a_dive_is_expirable_once_its_day_is_over_and_the_grace_has_passed() {
        const S: u64 = NS_PER_SECOND;
        let day = 20_401u32;
        let midnight = u64::from(day + 1) * DAY_SECONDS * S;
        let late = dive_started(day, midnight - 3600 * S); // 23:00
        let early = dive_started(day, midnight - 14 * 3600 * S); // 10:00
        assert_eq!((day_of(late.started_at), day_of(midnight)), (day, day + 1));
        // The same day: nobody can expire either.
        assert_eq!(expire_refusal(&late, midnight - S).as_deref(), Some("the dive's day is not over yet"));
        assert_eq!(expire_refusal(&early, midnight - S).as_deref(), Some("the dive's day is not over yet"));
        // One second into the next day the 10:00 dive can go; the 23:00 one cannot.
        assert_eq!(expire_refusal(&early, midnight + S), None);
        let refused = expire_refusal(&late, midnight + S).expect("a dive started at 23:00 is refused at 00:00:01");
        assert!(refused.starts_with("the dive started less than"), "{refused}");
        // ...until EXPIRE_GRACE_SECONDS after its start.
        let ready = late.started_at + EXPIRE_GRACE_SECONDS * S;
        assert!(expire_refusal(&late, ready - S).is_some());
        assert_eq!(expire_refusal(&late, ready), None);
    }

    /// An expired dive pays what a quit pays: the grave and the burn equal what the rules give for
    /// an empty log (a quit at the start) with the same loadout.
    #[test]
    fn an_expired_dive_pays_the_death_toll_like_a_quit() {
        use ls_core::{ITEM_BLADE, ITEM_GEM, ITEM_GOLD, ITEM_PLATE, ITEM_POTION};
        let loadouts = [
            vec![(ITEM_GOLD, 57), (ITEM_BLADE, 2)],
            vec![(ITEM_GOLD, 9)],
            vec![(ITEM_PLATE, 1)],
            vec![(ITEM_POTION, 4), (ITEM_GOLD, 1_000_003), (ITEM_GEM, 1)],
            vec![],
        ];
        for loadout in loadouts {
            let entry = DiveEntry { start_depth: START_DEPTH, inventory: loadout.clone(), graves: vec![] };
            let quit = ls_core::run_dive(7, &entry, &[]);
            assert!(quit.quit && quit.died);
            assert_eq!(expire_split(&loadout), (quit.dropped_into_grave, quit.burned), "{loadout:?}");
        }
        assert_eq!(expire_split(&[(ITEM_GOLD, 57)]), (vec![(ITEM_GOLD, 52)], vec![(ITEM_GOLD, 5)]));
    }
}
