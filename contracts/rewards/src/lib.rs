//! # Trivela Rewards Contract
//!
//! On-chain points and rewards for the Trivela campaign platform.
//! Tracks user balances and allows claiming rewards.
//!
//! Events:
//! - `credit`: topics `(credit, user)`, data `amount: u64`
//! - `claim`: topics `(claim, user)`, data `amount: u64`
//! - `transfer`: topics `(transfer, from, to)`, data `amount: u64`
//! - `paused`: topics `(paused,)`, data `is_paused: bool`
//! - `max_credit_per_call`: topics `(mxcredit,)`, data `max_amount: u64`
//! - `campaign_multiplier`: topics `(multset, campaign_id)`, data `multiplier_bps: u32`
//! - `pruned`: topics `(pruned, kind)`, data `count: u32`
//!
//! ## Storage pruning
//!
//! Multisig nonce records are not bumped indefinitely on Soroban;
//! [`RewardsContract::prune_used_nonces`] lets anyone reclaim storage for
//! nonces past their TTL, in capped batches. [`RewardsContract::storage_stats`]
//! reports current usage for monitoring.
//!
//! ## Co-admin multisig
//!
//! `set_paused` is a critical operation: once a threshold is configured via
//! `set_multisig_threshold`, it requires at least that many valid co-admin
//! signatures (registered via `add_co_admin`) over `(op, nonce, args_hash)`,
//! verified with ed25519. The nonce is consumed on use regardless of how many
//! signers participated.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contractmeta, symbol_short, Address, Bytes, BytesN,
    Env, Symbol, Vec,
};

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    Overflow = 1,
    InsufficientBalance = 2,
    Unauthorized = 3,
    ContractPaused = 4,
    CreditLimitExceeded = 5,
    UnsupportedMigration = 6,
    InvalidMultiplier = 7,
    InvalidThreshold = 8,
    InsufficientSignatures = 9,
    NonceReused = 10,
    DuplicateSigner = 11,
    UnknownSigner = 12,
}

contractmeta!(
    key = "Description",
    val = "Trivela campaign rewards and points"
);

const ADMIN: Symbol = symbol_short!("admin");
const BALANCE: Symbol = symbol_short!("balance");
const CLAIMED: Symbol = symbol_short!("claimed");
const METADATA: Symbol = symbol_short!("metadata");
const PAUSED: Symbol = symbol_short!("paused");
const CREDIT_EVENT: Symbol = symbol_short!("credit");
const CLAIM_EVENT: Symbol = symbol_short!("claim");
const TRANSFER_EVENT: Symbol = symbol_short!("transfer");
const PAUSED_EVENT: Symbol = symbol_short!("paused");
const MAX_CREDIT_EVENT: Symbol = symbol_short!("mxcredit");
const CAMPAIGN_MULTIPLIER_EVENT: Symbol = symbol_short!("multset");
const MAX_CREDIT_PER_CALL: Symbol = symbol_short!("mxcredit");
const SCHEMA_VERSION: Symbol = symbol_short!("schema_v");
const CURRENT_SCHEMA_VERSION: u32 = 1;
const CAMPAIGN_MULTIPLIER: Symbol = symbol_short!("mult");
const TIERS: Symbol = symbol_short!("tiers");
const BPS_DENOMINATOR: u128 = 10_000;
const PRUNED_EVENT: Symbol = symbol_short!("pruned");

// ── multisig nonce storage (#451 / #454) ────────────────────────────────────
const NONCE_USED: Symbol = symbol_short!("msnonce");
const NONCE_REGISTRY: Symbol = symbol_short!("nreg");
const NONCE_CURSOR: Symbol = symbol_short!("ncursor");
/// Multisig nonces older than this many ledgers are eligible for pruning.
const NONCE_TTL_LEDGERS: u32 = 10_000;

// ── co-admin multisig (#454) ────────────────────────────────────────────────
const CO_ADMINS: Symbol = symbol_short!("coadmin");
const MULTISIG_THRESHOLD: Symbol = symbol_short!("msthresh");
const OP_SET_PAUSED: u32 = 1;

#[contract]
pub struct RewardsContract;

fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
    admin.require_auth();

    let stored_admin: Address = env.storage().instance().get(&ADMIN).unwrap();
    if &stored_admin != admin {
        return Err(Error::Unauthorized);
    }

    Ok(())
}

fn ensure_not_paused(env: &Env) -> Result<(), Error> {
    let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
    if paused {
        return Err(Error::ContractPaused);
    }

    Ok(())
}

/// Build the signed payload for a multisig operation: `sha256(op || nonce || args_hash)`.
/// `op` is a stable per-function discriminant used in place of the function
/// name string (Symbol byte access is not available in `no_std`).
fn multisig_message(env: &Env, op: u32, nonce: u64, args_hash: &BytesN<32>) -> Bytes {
    let mut buf = [0u8; 44];
    buf[0..4].copy_from_slice(&op.to_be_bytes());
    buf[4..12].copy_from_slice(&nonce.to_be_bytes());
    buf[12..44].copy_from_slice(&args_hash.to_array());
    Bytes::from_slice(env, &buf)
}

/// Verify at least `required` distinct co-admin signatures over
/// `(op, nonce, args_hash)`, then consume `nonce` for replay protection.
/// The nonce is consumed regardless of how many signers submitted.
fn verify_multisig(
    env: &Env,
    op: u32,
    args_hash: BytesN<32>,
    nonce: u64,
    signatures: &Vec<(Address, BytesN<64>)>,
) -> Result<(), Error> {
    let required: u32 = env.storage().instance().get(&MULTISIG_THRESHOLD).unwrap_or(0);
    if required == 0 {
        return Ok(());
    }

    let nonce_key = (NONCE_USED, nonce);
    if env.storage().instance().get::<_, u32>(&nonce_key).is_some() {
        return Err(Error::NonceReused);
    }

    let co_admins: Vec<(Address, BytesN<32>)> =
        env.storage().instance().get(&CO_ADMINS).unwrap_or(Vec::new(env));
    let message = multisig_message(env, op, nonce, &args_hash);

    let mut seen: Vec<Address> = Vec::new(env);
    for (signer, sig) in signatures.iter() {
        if seen.iter().any(|s| s == signer) {
            return Err(Error::DuplicateSigner);
        }
        let pubkey = co_admins
            .iter()
            .find_map(|(addr, key)| if addr == signer { Some(key) } else { None })
            .ok_or(Error::UnknownSigner)?;
        env.crypto().ed25519_verify(&pubkey, &message, &sig);
        seen.push_back(signer.clone());
    }

    if seen.len() < required {
        return Err(Error::InsufficientSignatures);
    }

    env.storage().instance().set(&nonce_key, &env.ledger().sequence());
    let mut registry: Vec<u64> = env.storage().instance().get(&NONCE_REGISTRY).unwrap_or(Vec::new(env));
    registry.push_back(nonce);
    env.storage().instance().set(&NONCE_REGISTRY, &registry);
    Ok(())
}

#[contractimpl]
impl RewardsContract {
    /// Initialize the rewards contract (admin).
    pub fn initialize(env: Env, admin: Address, name: Symbol, symbol: Symbol) -> Result<(), Error> {
        env.storage().instance().set(&ADMIN, &admin);
        env.storage().instance().set(&CLAIMED, &0u64);
        env.storage().instance().set(&METADATA, &(name, symbol));
        env.storage().instance().set(&PAUSED, &false);
        env.storage().instance().set(&MAX_CREDIT_PER_CALL, &0u64);
        env.storage()
            .instance()
            .set(&SCHEMA_VERSION, &CURRENT_SCHEMA_VERSION);
        Ok(())
    }

    /// Returns the active storage schema version for this contract.
    pub fn schema_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&SCHEMA_VERSION)
            .unwrap_or(CURRENT_SCHEMA_VERSION)
    }

    /// Migration entrypoint for future schema changes.
    ///
    /// Current behavior is intentionally idempotent for version `1`, so operational
    /// scripts can call this safely during deployments/upgrades.
    pub fn migrate(env: Env, admin: Address, target_version: u32) -> Result<u32, Error> {
        require_admin(&env, &admin)?;
        if target_version != CURRENT_SCHEMA_VERSION {
            return Err(Error::UnsupportedMigration);
        }
        env.storage()
            .instance()
            .set(&SCHEMA_VERSION, &CURRENT_SCHEMA_VERSION);
        env.storage().instance().extend_ttl(50, 100);
        Ok(CURRENT_SCHEMA_VERSION)
    }

    /// Set maximum amount allowed per single credit call (admin only).
    /// Set to 0 to disable the limit.
    pub fn set_max_credit_per_call(env: Env, admin: Address, max_amount: u64) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        env.storage()
            .instance()
            .set(&MAX_CREDIT_PER_CALL, &max_amount);
        env.events().publish((MAX_CREDIT_EVENT,), max_amount);
        env.storage().instance().extend_ttl(50, 100);
        Ok(())
    }

    /// Get maximum amount allowed per single credit call (0 means unlimited).
    pub fn max_credit_per_call(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&MAX_CREDIT_PER_CALL)
            .unwrap_or(0)
    }

    /// Set campaign-specific reward multiplier in basis points (admin only).
    /// Example: 10_000 = 1.0x, 12_500 = 1.25x, 5_000 = 0.5x.
    pub fn set_campaign_multiplier(
        env: Env,
        admin: Address,
        campaign_id: u64,
        multiplier_bps: u32,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        if multiplier_bps == 0 {
            return Err(Error::InvalidMultiplier);
        }
        env.storage()
            .instance()
            .set(&(CAMPAIGN_MULTIPLIER, campaign_id), &multiplier_bps);
        env.events()
            .publish((CAMPAIGN_MULTIPLIER_EVENT, campaign_id), multiplier_bps);
        env.storage().instance().extend_ttl(50, 100);
        Ok(())
    }

    /// Returns multiplier in basis points for campaign, defaults to 10_000.
    pub fn campaign_multiplier(env: Env, campaign_id: u64) -> u32 {
        env.storage()
            .instance()
            .get(&(CAMPAIGN_MULTIPLIER, campaign_id))
            .unwrap_or(10_000)
    }

    /// Get contract metadata (name and symbol).
    pub fn metadata(env: Env) -> (Symbol, Symbol) {
        env.storage()
            .instance()
            .get(&METADATA)
            .unwrap_or((symbol_short!("Trivela"), symbol_short!("TVL")))
    }

    /// Get the current points balance for a user.
    pub fn balance(env: Env, user: Address) -> u64 {
        env.storage().instance().get(&(BALANCE, user)).unwrap_or(0)
    }

    /// Credit points to a user.
    pub fn credit(env: Env, from: Address, user: Address, amount: u64) -> Result<u64, Error> {
        from.require_auth();
        ensure_not_paused(&env)?;

        let max_credit_per_call: u64 = env
            .storage()
            .instance()
            .get(&MAX_CREDIT_PER_CALL)
            .unwrap_or(0);
        if max_credit_per_call > 0 && amount > max_credit_per_call {
            return Err(Error::CreditLimitExceeded);
        }

        let key = (BALANCE, user.clone());
        let current: u64 = env.storage().instance().get(&key).unwrap_or(0);
        let new_balance = current.checked_add(amount).ok_or(Error::Overflow)?;
        env.storage().instance().set(&key, &new_balance);
        env.events().publish((CREDIT_EVENT, user), amount);
        env.storage().instance().extend_ttl(50, 100);
        Ok(new_balance)
    }

    /// Credit points using campaign multiplier. Rounding uses floor division:
    /// `adjusted = base_amount * multiplier_bps / 10_000`.
    pub fn credit_for_campaign(
        env: Env,
        from: Address,
        user: Address,
        campaign_id: u64,
        base_amount: u64,
    ) -> Result<u64, Error> {
        let multiplier_bps: u32 = env
            .storage()
            .instance()
            .get(&(CAMPAIGN_MULTIPLIER, campaign_id))
            .unwrap_or(10_000);
        if multiplier_bps == 0 {
            return Err(Error::InvalidMultiplier);
        }
        let adjusted_u128 = (base_amount as u128)
            .checked_mul(multiplier_bps as u128)
            .ok_or(Error::Overflow)?
            / BPS_DENOMINATOR;
        if adjusted_u128 > u64::MAX as u128 {
            return Err(Error::Overflow);
        }
        let adjusted = adjusted_u128 as u64;
        Self::credit(env, from, user, adjusted)
    }

    /// Credit points to multiple users in one call.
    pub fn batch_credit(
        env: Env,
        from: Address,
        recipients: Vec<(Address, u64)>,
    ) -> Result<(), Error> {
        from.require_auth();
        ensure_not_paused(&env)?;

        let mut staged = Vec::new(&env);

        for (user, amount) in recipients.iter() {
            let key = (BALANCE, user.clone());
            let current: u64 = env.storage().instance().get(&key).unwrap_or(0);
            let new_balance = current.checked_add(amount).ok_or(Error::Overflow)?;
            staged.push_back((user, new_balance));
        }

        for (user, new_balance) in staged.iter() {
            env.storage()
                .instance()
                .set(&(BALANCE, user.clone()), &new_balance);
        }

        // Emit credit event for each recipient
        for (user, amount) in recipients.iter() {
            env.events().publish((CREDIT_EVENT, user), amount);
        }

        env.storage().instance().extend_ttl(50, 100);
        Ok(())
    }

    /// Claim rewards for a user (reduces balance).
    pub fn claim(env: Env, user: Address, amount: u64) -> Result<u64, Error> {
        user.require_auth();
        ensure_not_paused(&env)?;

        let key = (BALANCE, user.clone());
        let current: u64 = env.storage().instance().get(&key).unwrap_or(0);
        let new_balance = current
            .checked_sub(amount)
            .ok_or(Error::InsufficientBalance)?;
        env.storage().instance().set(&key, &new_balance);

        let total: u64 = env.storage().instance().get(&CLAIMED).unwrap_or(0);
        env.storage()
            .instance()
            .set(&CLAIMED, &total.saturating_add(amount));

        env.events().publish((CLAIM_EVENT, user), amount);
        env.storage().instance().extend_ttl(50, 100);
        Ok(new_balance)
    }

    /// Get total claimed rewards (global stats).
    pub fn total_claimed(env: Env) -> u64 {
        env.storage().instance().get(&CLAIMED).unwrap_or(0)
    }

    /// Transfer points from one user to another (admin only).
    pub fn admin_transfer(
        env: Env,
        admin: Address,
        from: Address,
        to: Address,
        amount: u64,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;

        let from_key = (BALANCE, from.clone());
        let from_balance: u64 = env.storage().instance().get(&from_key).unwrap_or(0);
        let new_from_balance = from_balance
            .checked_sub(amount)
            .ok_or(Error::InsufficientBalance)?;
        env.storage().instance().set(&from_key, &new_from_balance);

        let to_key = (BALANCE, to.clone());
        let to_balance: u64 = env.storage().instance().get(&to_key).unwrap_or(0);
        let new_to_balance = to_balance.checked_add(amount).ok_or(Error::Overflow)?;
        env.storage().instance().set(&to_key, &new_to_balance);

        env.events().publish((TRANSFER_EVENT, from, to), amount);
        env.storage().instance().extend_ttl(50, 100);
        Ok(())
    }

    /// Pause the contract. Blocks credit and claim operations.
    ///
    /// This is a critical operation: when a multisig threshold is configured
    /// (see [`Self::set_multisig_threshold`]), `signatures` must contain at
    /// least `required` valid co-admin signatures over
    /// `(op, nonce, sha256(paused))`; otherwise pass an empty `Vec` and the
    /// legacy single-admin check applies (`nonce` is ignored in that case).
    pub fn set_paused(
        env: Env,
        admin: Address,
        nonce: u64,
        paused: bool,
        signatures: Vec<(Address, BytesN<64>)>,
    ) -> Result<(), Error> {
        let threshold: u32 = env.storage().instance().get(&MULTISIG_THRESHOLD).unwrap_or(0);
        if threshold > 0 {
            let mut buf = [0u8; 1];
            buf[0] = paused as u8;
            let args_hash = env.crypto().sha256(&Bytes::from_slice(&env, &buf)).into();
            verify_multisig(&env, OP_SET_PAUSED, args_hash, nonce, &signatures)?;
        } else {
            require_admin(&env, &admin)?;
        }
        env.storage().instance().set(&PAUSED, &paused);
        env.events().publish((PAUSED_EVENT,), paused);
        env.storage().instance().extend_ttl(50, 100);
        Ok(())
    }

    /// Check if contract is paused.
    pub fn is_paused(env: Env) -> bool {
        env.storage().instance().get(&PAUSED).unwrap_or(false)
    }

    /// Configure tiered reward distribution for a campaign (admin only).
    pub fn set_tiers(
        env: Env,
        admin: Address,
        campaign_id: u64,
        tiers: Vec<(u64, u64)>,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;

        let sorted = sort_tiers(&env, tiers);
        env.storage().instance().set(&(TIERS, campaign_id), &sorted);

        env.events().publish((Symbol::new(&env, "set_tiers"), campaign_id), ());
        env.storage().instance().extend_ttl(50, 100);
        Ok(())
    }

    /// Clear configured tiers for a campaign (admin only).
    pub fn clear_tiers(env: Env, admin: Address, campaign_id: u64) -> Result<(), Error> {
        require_admin(&env, &admin)?;

        env.storage().instance().remove(&(TIERS, campaign_id));

        env.events().publish((Symbol::new(&env, "clear_tiers"), campaign_id), ());
        env.storage().instance().extend_ttl(50, 100);
        Ok(())
    }

    /// Get points reward for a given rank under a campaign.
    pub fn get_tier_for_rank(env: Env, rank: u64, campaign_id: u64) -> u64 {
        let tiers_opt: Option<Vec<(u64, u64)>> = env.storage().instance().get(&(TIERS, campaign_id));
        if let Some(tiers) = tiers_opt {
            for (max_rank, points) in tiers.iter() {
                if max_rank > 0 {
                    if rank <= max_rank {
                        return points;
                    }
                } else if max_rank == 0 {
                    return points;
                }
            }
        }
        0
    }

    /// Credit points to a user based on their rank.
    pub fn credit_by_rank(
        env: Env,
        from: Address,
        user: Address,
        rank: u64,
        campaign_id: u64,
    ) -> Result<u64, Error> {
        from.require_auth();
        ensure_not_paused(&env)?;

        let points = Self::get_tier_for_rank(env.clone(), rank, campaign_id);
        let new_balance = Self::credit(env.clone(), from, user.clone(), points)?;

        env.events().publish(
            (Symbol::new(&env, "tier_credit"), user),
            (rank, points),
        );

        Ok(new_balance)
    }

    // ── nonce pruning (#451) ─────────────────────────────────────────────

    /// Remove multisig nonce records older than [`NONCE_TTL_LEDGERS`], up to
    /// `max_entries` per call. Callable by anyone since it only deletes
    /// stale data. Returns the number of entries pruned.
    pub fn prune_used_nonces(env: Env, max_entries: u32) -> u32 {
        let registry: Vec<u64> = env
            .storage()
            .instance()
            .get(&NONCE_REGISTRY)
            .unwrap_or(Vec::new(&env));
        let len = registry.len();
        if len == 0 || max_entries == 0 {
            return 0;
        }

        let now = env.ledger().sequence();
        let mut cursor: u32 = env.storage().instance().get(&NONCE_CURSOR).unwrap_or(0);
        if cursor >= len {
            cursor = 0;
        }

        let mut pruned = 0u32;
        let mut checked = 0u32;
        let mut idx = cursor;
        while checked < len && pruned < max_entries {
            let nonce = registry.get(idx).unwrap();
            let key = (NONCE_USED, nonce);
            if let Some(used_at) = env.storage().instance().get::<_, u32>(&key) {
                if now.saturating_sub(used_at) > NONCE_TTL_LEDGERS {
                    env.storage().instance().remove(&key);
                    pruned += 1;
                }
            }
            idx = (idx + 1) % len;
            checked += 1;
        }
        env.storage().instance().set(&NONCE_CURSOR, &idx);

        if pruned > 0 {
            env.events().publish((PRUNED_EVENT, symbol_short!("nonce")), pruned);
        }
        env.storage().instance().extend_ttl(50, 100);
        pruned
    }

    /// Storage stats for monitoring: `(participant_count, nonce_count, expired_estimate)`.
    /// `participant_count` is always `0` here; the rewards contract tracks
    /// balances, not participants. `expired_estimate` counts currently-stale
    /// nonce records.
    pub fn storage_stats(env: Env) -> (u64, u64, u64) {
        let registry: Vec<u64> = env
            .storage()
            .instance()
            .get(&NONCE_REGISTRY)
            .unwrap_or(Vec::new(&env));
        let nonce_count = registry.len() as u64;

        let now = env.ledger().sequence();
        let mut expired = 0u64;
        for nonce in registry.iter() {
            if let Some(used_at) = env.storage().instance().get::<_, u32>(&(NONCE_USED, nonce)) {
                if now.saturating_sub(used_at) > NONCE_TTL_LEDGERS {
                    expired += 1;
                }
            }
        }
        (0, nonce_count, expired)
    }

    // ── co-admin multisig (#454) ────────────────────────────────────────

    /// Register a co-admin's ed25519 public key for multisig verification
    /// (admin only). Overwrites the key if `co_admin` is already registered.
    pub fn add_co_admin(env: Env, admin: Address, co_admin: Address, pubkey: BytesN<32>) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        let mut co_admins: Vec<(Address, BytesN<32>)> =
            env.storage().instance().get(&CO_ADMINS).unwrap_or(Vec::new(&env));
        let mut found = false;
        for i in 0..co_admins.len() {
            let (addr, _) = co_admins.get(i).unwrap();
            if addr == co_admin {
                co_admins.set(i, (co_admin.clone(), pubkey.clone()));
                found = true;
                break;
            }
        }
        if !found {
            co_admins.push_back((co_admin, pubkey));
        }
        env.storage().instance().set(&CO_ADMINS, &co_admins);
        env.storage().instance().extend_ttl(50, 100);
        Ok(())
    }

    /// Remove a co-admin from the multisig signer set (admin only).
    pub fn remove_co_admin(env: Env, admin: Address, co_admin: Address) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        let co_admins: Vec<(Address, BytesN<32>)> =
            env.storage().instance().get(&CO_ADMINS).unwrap_or(Vec::new(&env));
        let mut remaining = Vec::new(&env);
        for (addr, pubkey) in co_admins.iter() {
            if addr != co_admin {
                remaining.push_back((addr, pubkey));
            }
        }
        env.storage().instance().set(&CO_ADMINS, &remaining);
        env.storage().instance().extend_ttl(50, 100);
        Ok(())
    }

    /// Set the M-of-N multisig threshold for critical operations (admin only).
    /// `required = 0` disables multisig (legacy single-admin auth applies).
    pub fn set_multisig_threshold(env: Env, admin: Address, required: u32) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        let co_admins: Vec<(Address, BytesN<32>)> =
            env.storage().instance().get(&CO_ADMINS).unwrap_or(Vec::new(&env));
        if required > co_admins.len() {
            return Err(Error::InvalidThreshold);
        }
        env.storage().instance().set(&MULTISIG_THRESHOLD, &required);
        env.storage().instance().extend_ttl(50, 100);
        Ok(())
    }

    /// Returns the configured M-of-N multisig threshold (0 = disabled).
    pub fn multisig_threshold(env: Env) -> u32 {
        env.storage().instance().get(&MULTISIG_THRESHOLD).unwrap_or(0)
    }
}

fn sort_tiers(env: &Env, tiers: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    let mut sorted = tiers.clone();
    let len = sorted.len();
    if len <= 1 {
        return sorted;
    }

    for i in 0..len {
        for j in 0..len - 1 - i {
            let (rank_a, points_a) = sorted.get(j).unwrap();
            let (rank_b, points_b) = sorted.get(j + 1).unwrap();

            let swap = if rank_a == 0 && rank_b != 0 {
                true
            } else if rank_a != 0 && rank_b != 0 && rank_a > rank_b {
                true
            } else {
                false
            };

            if swap {
                sorted.set(j, (rank_b, points_b));
                sorted.set(j + 1, (rank_a, points_a));
            }
        }
    }
    sorted
}


#[cfg(test)]
mod test;
