//! Last Server contract (gas spike).
//!
//! `settle_dive` re-runs `ls_core::run_dive` on the caller's seed and move log, then applies
//! the outcome: mints found items, hands over looted graves, and on death moves what the
//! player carried into a new grave held by the contract. Items are NEP-245 multi-tokens with
//! a minimal hand-written core (transfers, balances, supply, NEP-297 events).
//!
//! Spike scope: no NEP-145 storage deposits, no approvals, no `mt_transfer_call`, the seed is
//! chosen by the caller and the move log length is not capped.

use ls_core::{DiveEntry, DiveOutcome, Grave, ItemId};
use near_sdk::json_types::{Base64VecU8, U128, U64};
use near_sdk::serde::Serialize;
use near_sdk::store::{IterableMap, LookupMap};
use near_sdk::{env, near, require, AccountId, BorshStorageKey, PanicOnDefault};

#[derive(BorshStorageKey)]
#[near]
enum StorageKey {
    Balances,
    Supply,
    Graves,
}

/// A grave on the server: what a dead player left at a depth.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct GraveRecord {
    pub owner_id: AccountId,
    pub depth: u32,
    pub x: u8,
    pub y: u8,
    pub items: Vec<(ItemId, u32)>,
}

/// `graves` view row.
#[near(serializers = [json])]
pub struct GraveView {
    pub id: U64,
    #[serde(flatten)]
    pub grave: GraveRecord,
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
    /// (token_id, owner) -> balance
    balances: LookupMap<(String, AccountId), u128>,
    supply: LookupMap<String, u128>,
    token_ids: Vec<String>,
    graves: IterableMap<u64, GraveRecord>,
    next_grave_id: u64,
}

#[near]
impl Contract {
    #[init]
    pub fn new() -> Self {
        let token_ids = ls_core::ITEM_IDS.iter().map(|&id| token_name(id).to_string()).collect();
        Self {
            balances: LookupMap::new(StorageKey::Balances),
            supply: LookupMap::new(StorageKey::Supply),
            token_ids,
            graves: IterableMap::new(StorageKey::Graves),
            next_grave_id: 0,
        }
    }

    // ---------------------------------------------------------------------------------------
    // Game
    // ---------------------------------------------------------------------------------------

    /// Re-runs the dive on chain and applies its outcome. The caller only sends seed + moves.
    pub fn settle_dive(&mut self, seed: U64, moves: Base64VecU8) -> DiveOutcome {
        let player = env::predecessor_account_id();
        let me = env::current_account_id();
        let entry = self.entry_for(&player);
        let outcome = ls_core::run_dive(seed.0, &entry, &moves.0);

        // 1. New items from the dungeon are minted to the player.
        if !outcome.picked.is_empty() {
            let items = named(&outcome.picked);
            for (token, amount) in &items {
                self.add_balance(token, &player, *amount);
                let s = self.supply.get(token).copied().unwrap_or(0);
                self.supply.insert(token.clone(), s.checked_add(*amount).expect("supply overflow"));
            }
            emit_mint(&player, &items, Some("dive"));
        }

        // 2. Looted graves move from the contract to the player.
        if !outcome.looted_graves.is_empty() {
            let mut loot: Vec<(ItemId, u32)> = Vec::new();
            for &id in &outcome.looted_graves {
                let grave = self.graves.remove(&u64::from(id)).expect("looted grave exists");
                for (item, amount) in grave.items {
                    ls_core::add_item(&mut loot, item, amount);
                }
            }
            let items = named(&loot);
            self.move_tokens(&me, &player, &items);
            emit_transfer(&me, &player, &items, Some("grave loot"));
        }

        // 3. Death: the toll is burned, the rest goes into a new grave held by the contract.
        if outcome.died {
            if !outcome.burned.is_empty() {
                let items = named(&outcome.burned);
                for (token, amount) in &items {
                    self.sub_balance(token, &player, *amount);
                    let s = self.supply.get(token).copied().unwrap_or(0);
                    self.supply.insert(token.clone(), s.checked_sub(*amount).expect("supply underflow"));
                }
                emit_burn(&player, &items, Some("death toll"));
            }
            if !outcome.dropped_into_grave.is_empty() {
                let items = named(&outcome.dropped_into_grave);
                self.move_tokens(&player, &me, &items);
                emit_transfer(&player, &me, &items, Some("grave"));
                let (x, y) = outcome.death_pos.expect("death position");
                let id = self.next_grave_id;
                self.next_grave_id += 1;
                self.graves.insert(
                    id,
                    GraveRecord {
                        owner_id: player.clone(),
                        depth: outcome.final_depth,
                        x,
                        y,
                        items: outcome.dropped_into_grave.clone(),
                    },
                );
            }
        }
        outcome
    }

    /// Exactly what `settle_dive` will feed into `run_dive` for this account right now, so a
    /// client can simulate the dive locally with the same inputs.
    pub fn dive_entry(&self, account_id: AccountId) -> DiveEntry {
        self.entry_for(&account_id)
    }

    pub fn graves(&self, depth: u32) -> Vec<GraveView> {
        self.graves
            .iter()
            .filter(|(_, g)| g.depth == depth)
            .map(|(id, g)| GraveView { id: U64(*id), grave: g.clone() })
            .collect()
    }

    pub fn core_version(&self) -> String {
        format!("ls-core {} rules-fnv1a64 {}", ls_core::VERSION, ls_core::RULES_HASH)
    }

    // ---------------------------------------------------------------------------------------
    // NEP-245 core
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

impl Contract {
    fn known(&self, token_id: &str) -> bool {
        self.token_ids.iter().any(|t| t == token_id)
    }

    fn balance(&self, token_id: &str, owner: &AccountId) -> u128 {
        self.balances.get(&(token_id.to_string(), owner.clone())).copied().unwrap_or(0)
    }

    fn add_balance(&mut self, token_id: &str, owner: &AccountId, amount: u128) {
        let key = (token_id.to_string(), owner.clone());
        let b = self.balances.get(&key).copied().unwrap_or(0);
        self.balances.insert(key, b.checked_add(amount).expect("balance overflow"));
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

    fn move_tokens(&mut self, from: &AccountId, to: &AccountId, items: &[(String, u128)]) {
        for (token, amount) in items {
            self.sub_balance(token, from, *amount);
            self.add_balance(token, to, *amount);
        }
    }

    /// User-initiated transfer: validates inputs, moves balances, emits `mt_transfer`.
    fn checked_transfer(
        &mut self,
        sender: &AccountId,
        receiver: &AccountId,
        items: Vec<(String, u128)>,
        memo: Option<&str>,
    ) {
        require!(sender != receiver, "sender and receiver must differ");
        require!(!items.is_empty(), "nothing to transfer");
        for (token, amount) in &items {
            require!(self.known(token), "unknown token_id");
            require!(*amount > 0, "amount must be positive");
        }
        self.move_tokens(sender, receiver, &items);
        emit_transfer(sender, receiver, &items, memo);
    }

    /// The `run_dive` input for an account: its item balances and every grave on the server.
    fn entry_for(&self, account: &AccountId) -> DiveEntry {
        let mut inventory = Vec::new();
        for &id in ls_core::ITEM_IDS.iter() {
            let b = self.balance(token_name(id), account);
            if b > 0 {
                inventory.push((id, u32::try_from(b).unwrap_or(u32::MAX)));
            }
        }
        let mut graves = Vec::new();
        for (id, g) in self.graves.iter() {
            let id = u32::try_from(*id).expect("grave id fits u32");
            graves.push(Grave { id, depth: g.depth, x: g.x, y: g.y, items: g.items.clone() });
        }
        DiveEntry { start_depth: 1, inventory, graves }
    }
}

fn token_name(id: ItemId) -> &'static str {
    ls_core::item_name(id).expect("known item id")
}

fn named(items: &[(ItemId, u32)]) -> Vec<(String, u128)> {
    items.iter().map(|&(id, n)| (token_name(id).to_string(), u128::from(n))).collect()
}

// -------------------------------------------------------------------------------------------
// NEP-297 events, standard "nep245" version "1.0.0"
// -------------------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
struct Event<'a, T: Serialize> {
    standard: &'static str,
    version: &'static str,
    event: &'static str,
    data: &'a [T],
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

fn emit<T: Serialize>(event: &'static str, data: &[T]) {
    let e = Event { standard: "nep245", version: "1.0.0", event, data };
    let json = near_sdk::serde_json::to_string(&e).expect("event serializes");
    env::log_str(&format!("EVENT_JSON:{json}"));
}

fn split(items: &[(String, u128)]) -> (Vec<&str>, Vec<U128>) {
    items.iter().map(|(t, n)| (t.as_str(), U128(*n))).unzip()
}

fn emit_mint(owner: &AccountId, items: &[(String, u128)], memo: Option<&str>) {
    let (token_ids, amounts) = split(items);
    emit("mt_mint", &[MtMintLog { owner_id: owner, token_ids, amounts, memo }]);
}

fn emit_burn(owner: &AccountId, items: &[(String, u128)], memo: Option<&str>) {
    let (token_ids, amounts) = split(items);
    emit("mt_burn", &[MtBurnLog { owner_id: owner, authorized_id: None, token_ids, amounts, memo }]);
}

fn emit_transfer(from: &AccountId, to: &AccountId, items: &[(String, u128)], memo: Option<&str>) {
    let (token_ids, amounts) = split(items);
    emit(
        "mt_transfer",
        &[MtTransferLog {
            authorized_id: None,
            old_owner_id: from,
            new_owner_id: to,
            token_ids,
            amounts,
            memo,
        }],
    );
}
