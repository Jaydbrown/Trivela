//! Tests for the Trivela campaign contract.

use super::*;
use ed25519_dalek::{Signer, SigningKey};
use soroban_sdk::testutils::{Address as _, Events as _, Ledger};
use soroban_sdk::{vec, Address, Bytes, BytesN, IntoVal, Vec};

/// Generate a deterministic ed25519 keypair for multisig tests, keyed by a
/// single seed byte so each co-admin gets a distinct key.
fn gen_keypair(seed: u8) -> SigningKey {
    let bytes = [seed; 32];
    SigningKey::from_bytes(&bytes)
}

fn sign_op(
    env: &Env,
    signing_key: &SigningKey,
    op: u32,
    nonce: u64,
    args_hash: &BytesN<32>,
) -> BytesN<64> {
    let mut buf = [0u8; 44];
    buf[0..4].copy_from_slice(&op.to_be_bytes());
    buf[4..12].copy_from_slice(&nonce.to_be_bytes());
    buf[12..44].copy_from_slice(&args_hash.to_array());
    let sig = signing_key.sign(&buf);
    BytesN::from_array(env, &sig.to_bytes())
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn setup() -> (Env, Address, CampaignContractClient<'static>) {
    let env = Env::default();
    let contract_id = env.register_contract(None, CampaignContract);
    let client = CampaignContractClient::new(&env, &contract_id);
    (env, contract_id, client)
}

/// Empty proof + dummy leaf – used when no Merkle root is configured.
fn no_proof_args(env: &Env) -> (BytesN<32>, Vec<BytesN<32>>) {
    (BytesN::from_array(env, &[0u8; 32]), Vec::new(env))
}

/// Build a two-leaf Merkle tree and return `(root, proof_for_a, proof_for_b)`.
///
/// Tree:
/// ```text
///        root
///       /    \
///   leaf_a  leaf_b
/// ```
/// Pairs are hashed in sorted order (same as `hash_pair` in lib.rs).
fn build_two_leaf_tree(
    env: &Env,
    leaf_a: BytesN<32>,
    leaf_b: BytesN<32>,
) -> (BytesN<32>, Vec<BytesN<32>>, Vec<BytesN<32>>) {
    let (left, right) = if leaf_a <= leaf_b {
        (leaf_a.clone(), leaf_b.clone())
    } else {
        (leaf_b.clone(), leaf_a.clone())
    };
    let mut combined = [0u8; 64];
    combined[..32].copy_from_slice(&left.to_array());
    combined[32..].copy_from_slice(&right.to_array());
    let root: BytesN<32> = env
        .crypto()
        .sha256(&Bytes::from_slice(env, &combined))
        .into();

    // Proof for leaf_a is [leaf_b], proof for leaf_b is [leaf_a].
    (root, vec![env, leaf_b], vec![env, leaf_a])
}

// ── original tests (updated for new `leaf` + `proof` parameters) ─────────────

#[test]
fn test_initialize_and_active() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);
    assert!(client.is_active());
}

#[test]
fn test_register_participant() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);
    env.mock_all_auths();
    let (leaf, proof) = no_proof_args(&env);
    let registered = client.register(&participant, &leaf, &proof, &None);
    assert!(registered);
    assert!(client.is_participant(&participant));
}

#[test]
fn test_time_window_validation() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    client.set_window(&admin, &0, &100, &200);

    let (leaf, proof) = no_proof_args(&env);

    // Too early — exact error and no participant recorded.
    env.ledger().with_mut(|li| li.timestamp = 50);
    assert_eq!(
        client.try_register(&participant, &leaf, &proof, &None),
        Err(Ok(Error::OutsideTimeWindow))
    );
    assert!(!client.is_participant(&participant));
    assert_eq!(client.get_participant_count(), 0);

    // Within window — succeeds.
    env.ledger().with_mut(|li| li.timestamp = 150);
    assert!(client.register(&participant, &leaf, &proof, &None));
    assert_eq!(client.get_participant_count(), 1);

    // Too late — exact error and count unchanged.
    let p2 = Address::generate(&env);
    env.ledger().with_mut(|li| li.timestamp = 250);
    assert_eq!(
        client.try_register(&p2, &leaf, &proof, &None),
        Err(Ok(Error::OutsideTimeWindow))
    );
    assert!(!client.is_participant(&p2));
    assert_eq!(client.get_participant_count(), 1);
}

#[test]
fn test_register_participant_twice_returns_false() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    let (leaf, proof) = no_proof_args(&env);
    assert!(client.register(&participant, &leaf, &proof, &None));
    assert!(!client.register(&participant, &leaf, &proof, &None));
}

#[test]
fn test_set_active_only_by_admin() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let other = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();

    // Non-admin cannot toggle active flag and the flag stays unchanged.
    assert!(client.is_active());
    assert_eq!(
        client.try_set_active(&other, &0, &false),
        Err(Ok(Error::Unauthorized))
    );
    assert!(client.is_active());
    // Admin nonce is not consumed when the call fails authorization.
    assert_eq!(client.admin_nonce(), 0);

    // Admin succeeds and flips the flag.
    client.set_active(&admin, &0, &false);
    assert!(!client.is_active());
}

#[test]
fn test_register_when_inactive() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    client.set_active(&admin, &0, &false);

    let (leaf, proof) = no_proof_args(&env);
    assert_eq!(
        client.try_register(&participant, &leaf, &proof, &None),
        Err(Ok(Error::CampaignInactive))
    );
    // No participant was recorded and counter did not move.
    assert!(!client.is_participant(&participant));
    assert_eq!(client.get_participant_count(), 0);

    // Re-activating allows the same participant to register normally.
    client.set_active(&admin, &1, &true);
    assert!(client.register(&participant, &leaf, &proof, &None));
    assert!(client.is_participant(&participant));
    assert_eq!(client.get_participant_count(), 1);
}

#[test]
fn test_is_participant_for_unknown_address() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let unknown_a = Address::generate(&env);
    let unknown_b = Address::generate(&env);
    client.initialize(&admin);

    // Multiple unrelated addresses all return false on a fresh contract.
    assert!(!client.is_participant(&unknown_a));
    assert!(!client.is_participant(&unknown_b));

    // Registering one address does not affect the membership of the other.
    let registered = Address::generate(&env);
    env.mock_all_auths();
    let (leaf, proof) = no_proof_args(&env);
    assert!(client.register(&registered, &leaf, &proof, &None));

    assert!(client.is_participant(&registered));
    assert!(!client.is_participant(&unknown_a));
    assert!(!client.is_participant(&unknown_b));
}

#[test]
fn test_capacity_reached() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    client.set_max_cap(&admin, &0, &1);

    let p1 = Address::generate(&env);
    let p2 = Address::generate(&env);
    let (leaf, proof) = no_proof_args(&env);
    assert!(client.register(&p1, &leaf, &proof, &None));
    let result = client.try_register(&p2, &leaf, &proof, &None);
    assert_eq!(result, Err(Ok(Error::CapReached)));
}

// ── Merkle tests ──────────────────────────────────────────────────────────────

#[test]
fn test_merkle_root_not_set_by_default() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);
    assert!(client.get_merkle_root().is_none());
}

#[test]
fn test_set_merkle_root_only_by_admin() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let other = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    let dummy: BytesN<32> = BytesN::from_array(&env, &[1u8; 32]);
    let result = client.try_set_merkle_root(&other, &0, &dummy, &Vec::new(&env));
    assert_eq!(result, Err(Ok(Error::Unauthorized)));
}

#[test]
fn test_register_with_valid_merkle_proof() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let p1 = Address::generate(&env);
    let p2 = Address::generate(&env);
    client.initialize(&admin);

    // Build a two-leaf tree; each participant is associated with one leaf.
    let leaf1: BytesN<32> = BytesN::from_array(&env, &[0xAAu8; 32]);
    let leaf2: BytesN<32> = BytesN::from_array(&env, &[0xBBu8; 32]);
    let (root, proof1, proof2) = build_two_leaf_tree(&env, leaf1.clone(), leaf2.clone());

    env.mock_all_auths();
    client.set_merkle_root(&admin, &0, &root, &Vec::new(&env));
    assert_eq!(client.get_merkle_root(), Some(root));

    // Both allowlisted participants can register with their correct leaf + proof.
    assert!(client.register(&p1, &leaf1, &proof1, &None));
    assert!(client.register(&p2, &leaf2, &proof2, &None));
    assert!(client.is_participant(&p1));
    assert!(client.is_participant(&p2));
}

#[test]
fn test_register_rejected_with_invalid_proof() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let p2 = Address::generate(&env);
    client.initialize(&admin);

    let leaf1: BytesN<32> = BytesN::from_array(&env, &[0xAAu8; 32]);
    let leaf2: BytesN<32> = BytesN::from_array(&env, &[0xBBu8; 32]);
    let (root, _proof1, _proof2) = build_two_leaf_tree(&env, leaf1.clone(), leaf2.clone());

    env.mock_all_auths();
    client.set_merkle_root(&admin, &0, &root, &Vec::new(&env));

    // p2 supplies leaf2 but with a totally wrong proof sibling.
    let wrong_sibling: BytesN<32> = BytesN::from_array(&env, &[0xFFu8; 32]);
    let bad_proof = vec![&env, wrong_sibling];
    let result = client.try_register(&p2, &leaf2, &bad_proof, &None);
    assert_eq!(result, Err(Ok(Error::NotInAllowlist)));
}

#[test]
fn test_register_rejected_with_leaf_not_in_tree() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let p3 = Address::generate(&env);
    client.initialize(&admin);

    let leaf1: BytesN<32> = BytesN::from_array(&env, &[0xAAu8; 32]);
    let leaf2: BytesN<32> = BytesN::from_array(&env, &[0xBBu8; 32]);
    let (root, _proof1, proof2) = build_two_leaf_tree(&env, leaf1.clone(), leaf2.clone());

    env.mock_all_auths();
    client.set_merkle_root(&admin, &0, &root, &Vec::new(&env));

    // p3 supplies a leaf that is not in the tree at all.
    let unknown_leaf: BytesN<32> = BytesN::from_array(&env, &[0xCCu8; 32]);
    let result = client.try_register(&p3, &unknown_leaf, &proof2, &None);
    assert_eq!(result, Err(Ok(Error::NotInAllowlist)));
}

#[test]
fn test_register_rejected_with_empty_proof_when_root_set() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let p1 = Address::generate(&env);
    client.initialize(&admin);

    let leaf1: BytesN<32> = BytesN::from_array(&env, &[0xAAu8; 32]);
    let leaf2: BytesN<32> = BytesN::from_array(&env, &[0xBBu8; 32]);
    let (root, _proof1, _proof2) = build_two_leaf_tree(&env, leaf1.clone(), leaf2.clone());

    env.mock_all_auths();
    client.set_merkle_root(&admin, &0, &root, &Vec::new(&env));

    // Empty proof should fail when root is set – a leaf alone does not equal the root.
    let result = client.try_register(&p1, &leaf1, &Vec::new(&env), &None);
    assert_eq!(result, Err(Ok(Error::NotInAllowlist)));
}

#[test]
fn test_open_registration_when_no_root() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);

    // No root set – any leaf/proof is accepted.
    env.mock_all_auths();
    let (leaf, proof) = no_proof_args(&env);
    assert!(client.register(&participant, &leaf, &proof, &None));
}

#[test]
fn test_schema_version_and_migrate_entrypoint() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let other = Address::generate(&env);
    client.initialize(&admin);
    assert_eq!(client.schema_version(), 1);

    env.mock_all_auths();
    let migrated = client.migrate(&admin, &1);
    assert_eq!(migrated, 1);
    assert_eq!(client.schema_version(), 1);

    let unsupported = client.try_migrate(&admin, &2);
    assert_eq!(unsupported, Err(Ok(Error::UnsupportedMigration)));

    let unauthorized = client.try_migrate(&other, &1);
    assert_eq!(unauthorized, Err(Ok(Error::Unauthorized)));
}

#[test]
fn test_participant_count_increments_on_new_register_only() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let p1 = Address::generate(&env);
    client.initialize(&admin);
    env.mock_all_auths();

    let (leaf, proof) = no_proof_args(&env);
    assert_eq!(client.get_participant_count(), 0);
    assert!(client.register(&p1, &leaf, &proof, &None));
    assert_eq!(client.get_participant_count(), 1);
    assert!(!client.register(&p1, &leaf, &proof, &None));
    assert_eq!(client.get_participant_count(), 1);
}

// ── window: getters, validation, boundaries (issue #89) ─────────────────────

#[test]
fn test_get_window_default_is_open() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);

    // After initialize, the window is "open": [0, u64::MAX].
    assert_eq!(client.get_window(), (0, u64::MAX));
    assert!(client.is_within_window());
}

#[test]
fn test_get_window_after_set() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    client.set_window(&admin, &0, &1_000, &2_000);
    assert_eq!(client.get_window(), (1_000, 2_000));
}

#[test]
fn test_set_window_rejects_start_after_end() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    let nonce_before = client.admin_nonce();
    assert_eq!(
        client.try_set_window(&admin, &nonce_before, &500, &100),
        Err(Ok(Error::InvalidWindow))
    );

    // Window stays at default. The nonce increment performed inside
    // `require_admin_with_nonce` is rolled back together with all other
    // writes when the function returns `Err`, so the same nonce can be
    // re-used for a corrected call.
    assert_eq!(client.get_window(), (0, u64::MAX));
    assert_eq!(client.admin_nonce(), nonce_before);

    // Same nonce now succeeds with a valid window.
    client.set_window(&admin, &nonce_before, &100, &500);
    assert_eq!(client.get_window(), (100, 500));
    assert_eq!(client.admin_nonce(), nonce_before + 1);
}

#[test]
fn test_set_window_allows_equal_start_and_end() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    client.set_window(&admin, &0, &500, &500);
    assert_eq!(client.get_window(), (500, 500));

    // Single-instant window: register works exactly at the boundary.
    let (leaf, proof) = no_proof_args(&env);
    env.ledger().with_mut(|li| li.timestamp = 500);
    assert!(client.is_within_window());
    assert!(client.register(&participant, &leaf, &proof, &None));
}

#[test]
fn test_register_at_window_boundaries() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    client.set_window(&admin, &0, &100, &200);
    let (leaf, proof) = no_proof_args(&env);

    // timestamp == start: inclusive lower bound.
    let p_start = Address::generate(&env);
    env.ledger().with_mut(|li| li.timestamp = 100);
    assert!(client.is_within_window());
    assert!(client.register(&p_start, &leaf, &proof, &None));

    // timestamp == end: inclusive upper bound.
    let p_end = Address::generate(&env);
    env.ledger().with_mut(|li| li.timestamp = 200);
    assert!(client.is_within_window());
    assert!(client.register(&p_end, &leaf, &proof, &None));

    // One past end: rejected.
    let p_after = Address::generate(&env);
    env.ledger().with_mut(|li| li.timestamp = 201);
    assert!(!client.is_within_window());
    assert_eq!(
        client.try_register(&p_after, &leaf, &proof, &None),
        Err(Ok(Error::OutsideTimeWindow))
    );
}

#[test]
fn test_set_window_emits_event() {
    let (env, contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    client.set_window(&admin, &0, &100, &200);

    assert_eq!(
        env.events().all(),
        vec![
            &env,
            (
                contract_id.clone(),
                vec![&env, SET_WINDOW_EVENT.into_val(&env)],
                (100u64, 200u64).into_val(&env)
            )
        ]
    );
}

// ── extra coverage for #91 ───────────────────────────────────────────────────

#[test]
fn test_set_active_emits_event_and_is_idempotent() {
    let (env, contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();

    // Toggle off — flag flips and a single event is emitted.
    // (`env.events().all()` reflects events from the most recent invocation,
    // so we assert it before any further client calls.)
    client.set_active(&admin, &0, &false);
    assert_eq!(
        env.events().all(),
        vec![
            &env,
            (
                contract_id.clone(),
                vec![&env, SET_ACTIVE_EVENT.into_val(&env)],
                false.into_val(&env)
            )
        ]
    );

    // Setting to the same value is allowed (idempotent) and still emits.
    client.set_active(&admin, &1, &false);
    assert_eq!(
        env.events().all(),
        vec![
            &env,
            (
                contract_id,
                vec![&env, SET_ACTIVE_EVENT.into_val(&env)],
                false.into_val(&env)
            )
        ]
    );
    assert!(!client.is_active());
}

#[test]
fn test_register_unauthorized_other_address_does_not_persist() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    let (leaf, proof) = no_proof_args(&env);
    assert!(client.register(&participant, &leaf, &proof, &None));

    // Sanity: a brand-new address is not silently registered as a side
    // effect of someone else's register call.
    let bystander = Address::generate(&env);
    assert!(!client.is_participant(&bystander));
    assert_eq!(client.get_participant_count(), 1);
}

#[test]
fn test_admin_nonce_replay_protection() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);
    env.mock_all_auths();

    assert_eq!(client.admin_nonce(), 0);
    client.set_active(&admin, &0, &false);
    assert_eq!(client.admin_nonce(), 1);

    let replay = client.try_set_active(&admin, &0, &true);
    assert_eq!(replay, Err(Ok(Error::InvalidAdminNonce)));

    client.set_active(&admin, &1, &true);
    assert_eq!(client.admin_nonce(), 2);
}

#[test]
fn test_deregister_success_and_re_register() {
    let (env, contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    let (leaf, proof) = no_proof_args(&env);

    // Register participant
    assert!(client.register(&participant, &leaf, &proof, &None));
    assert!(client.is_participant(&participant));
    assert_eq!(client.get_participant_count(), 1);

    // Deregister participant
    assert!(client.deregister(&participant));
    assert!(!client.is_participant(&participant));
    assert_eq!(client.get_participant_count(), 0);

    // Check deregister event
    let register_event = Symbol::new(&env, "register");
    let deregister_event = Symbol::new(&env, "deregister");
    assert_eq!(
        env.events().all(),
        vec![
            &env,
            (
                contract_id.clone(),
                vec![&env, register_event.into_val(&env), participant.clone().into_val(&env)],
                ().into_val(&env)
            ),
            (
                contract_id.clone(),
                vec![&env, deregister_event.into_val(&env), participant.clone().into_val(&env)],
                ().into_val(&env)
            )
        ]
    );

    // Re-register works
    assert!(client.register(&participant, &leaf, &proof, &None));
    assert!(client.is_participant(&participant));
    assert_eq!(client.get_participant_count(), 1);
}

#[test]
fn test_admin_deregister() {
    let (env, contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    let (leaf, proof) = no_proof_args(&env);

    // Register participant
    assert!(client.register(&participant, &leaf, &proof, &None));
    assert!(client.is_participant(&participant));
    assert_eq!(client.get_participant_count(), 1);

    // Admin deregister
    assert!(client.admin_deregister(&admin, &0, &participant));
    assert!(!client.is_participant(&participant));
    assert_eq!(client.get_participant_count(), 0);

    // Check deregister event
    let register_event = Symbol::new(&env, "register");
    let deregister_event = Symbol::new(&env, "deregister");
    assert_eq!(
        env.events().all(),
        vec![
            &env,
            (
                contract_id.clone(),
                vec![&env, register_event.into_val(&env), participant.clone().into_val(&env)],
                ().into_val(&env)
            ),
            (
                contract_id.clone(),
                vec![&env, deregister_event.into_val(&env), participant.clone().into_val(&env)],
                ().into_val(&env)
            )
        ]
    );

    // Call admin deregister again for same participant (should return false and not panic)
    assert!(!client.admin_deregister(&admin, &1, &participant));
    assert_eq!(client.get_participant_count(), 0);
}

#[test]
fn test_deregister_liveness_checks() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);

    env.mock_all_auths();
    let (leaf, proof) = no_proof_args(&env);

    // Register
    client.register(&participant, &leaf, &proof, &None);

    // Case 1: end_time != u64::MAX and now > end_time
    client.set_window(&admin, &0, &100, &200);
    env.ledger().with_mut(|li| li.timestamp = 250);
    assert_eq!(
        client.try_deregister(&participant),
        Err(Ok(Error::OutsideTimeWindow))
    );

    // Reset window to u64::MAX but campaign inactive
    client.set_window(&admin, &1, &100, &u64::MAX);
    client.set_active(&admin, &2, &false);
    env.ledger().with_mut(|li| li.timestamp = 250);
    assert_eq!(
        client.try_deregister(&participant),
        Err(Ok(Error::CampaignInactive))
    );

    // Admin deregister bypasses all these checks
    assert!(client.admin_deregister(&admin, &3, &participant));
    assert!(!client.is_participant(&participant));
}

// ── storage pruning (#451) ────────────────────────────────────────────────────

#[test]
fn test_prune_expired_participants_empty_is_noop() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);

    assert_eq!(client.prune_expired_participants(&10), 0);
}

#[test]
fn test_prune_expired_participants_removes_expired_entries() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let p1 = Address::generate(&env);
    let p2 = Address::generate(&env);
    client.initialize(&admin);
    env.mock_all_auths();

    let (leaf, proof) = no_proof_args(&env);
    assert!(client.register(&p1, &leaf, &proof, &None));
    assert!(client.register(&p2, &leaf, &proof, &None));
    assert_eq!(client.get_participant_count(), 2);

    // Advance past the participant TTL window.
    env.ledger()
        .with_mut(|li| li.sequence_number += PARTICIPANT_TTL_LEDGERS + 1);

    let pruned = client.prune_expired_participants(&10);
    assert_eq!(pruned, 2);
    assert_eq!(client.get_participant_count(), 0);
    assert!(!client.is_participant(&p1));
    assert!(!client.is_participant(&p2));

    // Pruning again finds nothing left to remove.
    assert_eq!(client.prune_expired_participants(&10), 0);
}

#[test]
fn test_prune_expired_participants_respects_max_entries_cap() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);
    env.mock_all_auths();

    let (leaf, proof) = no_proof_args(&env);
    for _ in 0..5 {
        let p = Address::generate(&env);
        assert!(client.register(&p, &leaf, &proof, &None));
    }
    assert_eq!(client.get_participant_count(), 5);

    env.ledger()
        .with_mut(|li| li.sequence_number += PARTICIPANT_TTL_LEDGERS + 1);

    // Capped: only 2 entries removed per call even though 5 are expired.
    assert_eq!(client.prune_expired_participants(&2), 2);
    assert_eq!(client.get_participant_count(), 3);
    assert_eq!(client.prune_expired_participants(&2), 2);
    assert_eq!(client.get_participant_count(), 1);
    assert_eq!(client.prune_expired_participants(&2), 1);
    assert_eq!(client.get_participant_count(), 0);
}

#[test]
fn test_storage_stats_tracks_participants_and_expired_estimate() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let p1 = Address::generate(&env);
    client.initialize(&admin);
    env.mock_all_auths();

    let (leaf, proof) = no_proof_args(&env);
    assert!(client.register(&p1, &leaf, &proof, &None));

    let (participant_count, _nonce_count, expired) = client.storage_stats();
    assert_eq!(participant_count, 1);
    assert_eq!(expired, 0);

    env.ledger()
        .with_mut(|li| li.sequence_number += PARTICIPANT_TTL_LEDGERS + 1);

    let (participant_count, _nonce_count, expired) = client.storage_stats();
    assert_eq!(participant_count, 1);
    assert_eq!(expired, 1);
}

// ── invite-only registration (#452) ───────────────────────────────────────────

#[test]
fn test_register_without_code_in_invite_only_mode_fails() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);
    env.mock_all_auths();

    client.set_invite_only(&admin, &0, &true);
    assert!(client.is_invite_only());

    let (leaf, proof) = no_proof_args(&env);
    assert_eq!(
        client.try_register(&participant, &leaf, &proof, &None),
        Err(Ok(Error::InviteCodeRequired))
    );
    assert!(!client.is_participant(&participant));
}

#[test]
fn test_register_with_invalid_code_fails() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);
    env.mock_all_auths();

    client.set_invite_only(&admin, &0, &true);

    let real_code = Bytes::from_slice(&env, b"correct-code");
    let real_hash: BytesN<32> = env.crypto().sha256(&real_code).into();
    client.issue_invite(&admin, &1, &real_hash);

    let wrong_code = Bytes::from_slice(&env, b"wrong-code");
    let (leaf, proof) = no_proof_args(&env);
    assert_eq!(
        client.try_register(&participant, &leaf, &proof, &Some(wrong_code)),
        Err(Ok(Error::InvalidInviteCode))
    );
    assert!(!client.is_participant(&participant));
}

#[test]
fn test_register_with_valid_code_succeeds_then_second_use_fails() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let p1 = Address::generate(&env);
    let p2 = Address::generate(&env);
    client.initialize(&admin);
    env.mock_all_auths();

    client.set_invite_only(&admin, &0, &true);

    let code = Bytes::from_slice(&env, b"my-invite-code");
    let hash: BytesN<32> = env.crypto().sha256(&code).into();
    client.issue_invite(&admin, &1, &hash);
    assert!(!client.invite_used(&hash));

    let (leaf, proof) = no_proof_args(&env);
    assert!(client.register(&p1, &leaf, &proof, &Some(code.clone())));
    assert!(client.is_participant(&p1));
    assert!(client.invite_used(&hash));

    // Second redemption of the same single-use code fails.
    assert_eq!(
        client.try_register(&p2, &leaf, &proof, &Some(code)),
        Err(Ok(Error::InviteAlreadyUsed))
    );
    assert!(!client.is_participant(&p2));
}

#[test]
fn test_revoke_invite_removes_it() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    let participant = Address::generate(&env);
    client.initialize(&admin);
    env.mock_all_auths();

    client.set_invite_only(&admin, &0, &true);
    let code = Bytes::from_slice(&env, b"revoke-me");
    let hash: BytesN<32> = env.crypto().sha256(&code).into();
    client.issue_invite(&admin, &1, &hash);
    client.revoke_invite(&admin, &2, &hash);

    let (leaf, proof) = no_proof_args(&env);
    assert_eq!(
        client.try_register(&participant, &leaf, &proof, &Some(code)),
        Err(Ok(Error::InvalidInviteCode))
    );
}

// ── co-admin multisig for set_merkle_root (#454) ──────────────────────────────

#[test]
fn test_multisig_2_of_3_one_signature_fails() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);
    env.mock_all_auths();

    let co1 = Address::generate(&env);
    let co2 = Address::generate(&env);
    let co3 = Address::generate(&env);
    let key1 = gen_keypair(1);
    let key2 = gen_keypair(2);
    let key3 = gen_keypair(3);
    let pub1 = BytesN::from_array(&env, &key1.verifying_key().to_bytes());
    let pub2 = BytesN::from_array(&env, &key2.verifying_key().to_bytes());
    let pub3 = BytesN::from_array(&env, &key3.verifying_key().to_bytes());

    client.add_co_admin(&admin, &0, &co1, &pub1);
    client.add_co_admin(&admin, &1, &co2, &pub2);
    client.add_co_admin(&admin, &2, &co3, &pub3);
    client.set_multisig_threshold(&admin, &3, &2);
    assert_eq!(client.multisig_threshold(), 2);

    let root: BytesN<32> = BytesN::from_array(&env, &[7u8; 32]);
    let args_hash: BytesN<32> = env
        .crypto()
        .sha256(&Bytes::from_slice(&env, &root.to_array()))
        .into();
    let nonce = 42u64;
    let sig1 = sign_op(&env, &key1, OP_SET_MERKLE_ROOT, nonce, &args_hash);

    let signatures = vec![&env, (co1.clone(), sig1)];
    let result = client.try_set_merkle_root(&admin, &nonce, &root, &signatures);
    assert_eq!(result, Err(Ok(Error::InsufficientSignatures)));
    assert!(client.get_merkle_root().is_none());
}

#[test]
fn test_multisig_2_of_3_two_signatures_succeed_and_nonce_replay_fails() {
    let (env, _contract_id, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin);
    env.mock_all_auths();

    let co1 = Address::generate(&env);
    let co2 = Address::generate(&env);
    let co3 = Address::generate(&env);
    let key1 = gen_keypair(1);
    let key2 = gen_keypair(2);
    let key3 = gen_keypair(3);
    let pub1 = BytesN::from_array(&env, &key1.verifying_key().to_bytes());
    let pub2 = BytesN::from_array(&env, &key2.verifying_key().to_bytes());
    let pub3 = BytesN::from_array(&env, &key3.verifying_key().to_bytes());

    client.add_co_admin(&admin, &0, &co1, &pub1);
    client.add_co_admin(&admin, &1, &co2, &pub2);
    client.add_co_admin(&admin, &2, &co3, &pub3);
    client.set_multisig_threshold(&admin, &3, &2);

    let root: BytesN<32> = BytesN::from_array(&env, &[7u8; 32]);
    let args_hash: BytesN<32> = env
        .crypto()
        .sha256(&Bytes::from_slice(&env, &root.to_array()))
        .into();
    let nonce = 42u64;
    let sig1 = sign_op(&env, &key1, OP_SET_MERKLE_ROOT, nonce, &args_hash);
    let sig2 = sign_op(&env, &key2, OP_SET_MERKLE_ROOT, nonce, &args_hash);

    let signatures = vec![&env, (co1.clone(), sig1), (co2.clone(), sig2)];
    client.set_merkle_root(&admin, &nonce, &root, &signatures);
    assert_eq!(client.get_merkle_root(), Some(root.clone()));

    // Replaying the same nonce (even with valid signatures) fails.
    let root2: BytesN<32> = BytesN::from_array(&env, &[9u8; 32]);
    let args_hash2: BytesN<32> = env
        .crypto()
        .sha256(&Bytes::from_slice(&env, &root2.to_array()))
        .into();
    let sig1b = sign_op(&env, &key1, OP_SET_MERKLE_ROOT, nonce, &args_hash2);
    let sig2b = sign_op(&env, &key2, OP_SET_MERKLE_ROOT, nonce, &args_hash2);
    let signatures2 = vec![&env, (co1, sig1b), (co2, sig2b)];
    let result = client.try_set_merkle_root(&admin, &nonce, &root2, &signatures2);
    assert_eq!(result, Err(Ok(Error::NonceReused)));
    assert_eq!(client.get_merkle_root(), Some(root));
}

