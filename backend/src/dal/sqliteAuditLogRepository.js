import Database from 'better-sqlite3';

const SCHEMA = `
CREATE TABLE IF NOT EXISTS audit_logs (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  actor      TEXT    NOT NULL,
  action     TEXT    NOT NULL,
  entity     TEXT    NOT NULL,
  entity_id  TEXT,
  diff       TEXT,
  created_at TEXT    NOT NULL
);
`;

function rowToAuditLog(row) {
  return {
    id: String(row.id),
    actor: row.actor,
    action: row.action,
    entity: row.entity,
    entityId: row.entity_id ?? null,
    diff: row.diff ? JSON.parse(row.diff) : null,
    timestamp: row.created_at,
  };
}

export function createSqliteAuditLogRepository({ dbPath = ':memory:' } = {}) {
  const db = new Database(dbPath);
  db.exec(SCHEMA);

  function create({ actor, action, entity, entityId = null, diff = null, timestamp = null }) {
    const createdAt = timestamp ?? new Date().toISOString();
    const diffJson = diff ? JSON.stringify(diff) : null;
    const info = db
      .prepare(
        'INSERT INTO audit_logs (actor, action, entity, entity_id, diff, created_at) VALUES (?, ?, ?, ?, ?, ?)',
      )
      .run(actor, action, entity, entityId, diffJson, createdAt);
    return db
      .prepare('SELECT * FROM audit_logs WHERE id = ?')
      .get(info.lastInsertRowid);
  }

  function list({ entity, entityId, action } = {}) {
    const filters = [];
    const values = [];

    if (entity) {
      filters.push('entity = ?');
      values.push(entity);
    }
    if (entityId) {
      filters.push('entity_id = ?');
      values.push(String(entityId));
    }
    if (action) {
      filters.push('action = ?');
      values.push(action);
    }

    const where = filters.length > 0 ? `WHERE ${filters.join(' AND ')}` : '';
    return db
      .prepare(`SELECT * FROM audit_logs ${where} ORDER BY id DESC`)
      .all(...values)
      .map(rowToAuditLog);
  }

  return {
    create,
    list,
  };
}

