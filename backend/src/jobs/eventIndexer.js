/**
 * Event indexer for Trivela Soroban contract events.
 *
 * Subscribes to on-chain events and persists them to the database.
 * Snapshot events store a ledger reference so off-chain tools can
 * reconstruct user balances at that point using Horizon getLedgerEntries.
 */

export function createEventIndexer({ db, rpcPool, logger = console } = {}) {
  const handlers = {
    credit: handleCreditEvent,
    claim: handleClaimEvent,
    snapshot: handleSnapshotEvent,
    vcredit: handleVestedCreditEvent,
    vclaim: handleVestedClaimEvent,
  };

  async function processEvent(event) {
    const topic = event.topic?.[0];
    const handler = handlers[topic];
    if (!handler) return;
    try {
      await handler(event, db);
    } catch (err) {
      logger.error?.(`eventIndexer:error topic=${topic}`, err);
    }
  }

  async function poll(contractId, cursor) {
    const rpc = await rpcPool.acquire();
    try {
      const { events, nextCursor } = await rpc.getEvents({
        contractId,
        cursor,
        limit: 200,
      });
      for (const event of events) {
        await processEvent(event);
      }
      return nextCursor;
    } finally {
      rpcPool.release(rpc);
    }
  }

  return { processEvent, poll };
}

async function handleCreditEvent(event, db) {
  const user = event.topic?.[1];
  const amount = BigInt(event.data ?? 0);
  await db.run(
    `INSERT OR IGNORE INTO balances (user) VALUES (?)
     ON CONFLICT(user) DO UPDATE SET balance = balance + ?`,
    [user, amount.toString()],
  );
  await db.run(
    `INSERT INTO credit_events (user, amount, ledger, tx_hash) VALUES (?, ?, ?, ?)`,
    [user, amount.toString(), event.ledger, event.txHash],
  );
}

async function handleClaimEvent(event, db) {
  const user = event.topic?.[1];
  const amount = BigInt(event.data ?? 0);
  await db.run(
    `UPDATE balances SET balance = balance - ? WHERE user = ?`,
    [amount.toString(), user],
  );
  await db.run(
    `INSERT INTO claim_events (user, amount, ledger, tx_hash) VALUES (?, ?, ?, ?)`,
    [user, amount.toString(), event.ledger, event.txHash],
  );
}

/**
 * Index a snapshot event.
 *
 * The contract stores only a ledger reference — not a full balance copy.
 * Off-chain consumers can call Horizon getLedgerEntries with this ledger
 * number to reconstruct all user balances at that exact point in time.
 *
 * Pattern:
 *   1. Read `snapshot_ledger` from this event.
 *   2. Fetch all `BALANCE` storage entries at `snapshot_ledger` from Horizon.
 *   3. Persist the reconstructed balances to `snapshot_balances`.
 */
async function handleSnapshotEvent(event, db) {
  const snapshotId = BigInt(event.topic?.[1] ?? 0);
  const snapshotLedger = BigInt(event.data ?? 0);
  await db.run(
    `INSERT OR REPLACE INTO snapshots (snapshot_id, ledger_number, recorded_at)
     VALUES (?, ?, ?)`,
    [snapshotId.toString(), snapshotLedger.toString(), Date.now()],
  );
}

async function handleVestedCreditEvent(event, db) {
  const user = event.topic?.[1];
  const [vestId, total] = Array.isArray(event.data) ? event.data : [0, 0];
  await db.run(
    `INSERT INTO vesting_schedules (user, vest_id, total, ledger, tx_hash)
     VALUES (?, ?, ?, ?, ?)`,
    [user, String(vestId), String(total), event.ledger, event.txHash],
  );
}

async function handleVestedClaimEvent(event, db) {
  const user = event.topic?.[1];
  const [vestId, amount] = Array.isArray(event.data) ? event.data : [0, 0];
  await db.run(
    `INSERT INTO vested_claim_events (user, vest_id, amount, ledger, tx_hash)
     VALUES (?, ?, ?, ?, ?)`,
    [user, String(vestId), String(amount), event.ledger, event.txHash],
  );
}
