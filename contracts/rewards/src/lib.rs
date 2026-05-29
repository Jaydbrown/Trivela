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

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contractmeta, symbol_short, Address, Env, Symbol, Vec,
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

    /// Pause the contract (admin only). Blocks credit and claim operations.
    pub fn set_paused(env: Env, admin: Address, paused: bool) -> Result<(), Error> {
        require_admin(&env, &admin)?;
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
