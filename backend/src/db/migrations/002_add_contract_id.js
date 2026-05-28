export const version = 2;
export const description = 'Add contract_id for on-chain campaign anchoring';

export function up(db) {
  db.exec(`
    ALTER TABLE campaigns ADD COLUMN contract_id TEXT;

    CREATE INDEX IF NOT EXISTS idx_campaigns_contract_id ON campaigns(contract_id);
  `);
}
