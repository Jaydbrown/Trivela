// @ts-check
import { Router } from 'express';
import { pipeline } from 'node:stream/promises';
import { Readable } from 'node:stream';

const EXPORT_RATE_LIMIT_WINDOW_MS = 60 * 60 * 1000; // 1 hour
const EXPORT_RATE_LIMIT_MAX = 5;

/**
 * In-memory store for export rate limiting per campaign.
 * Key: `${campaignId}:${apiKeyId}`, value: { count, resetAt }
 * @type {Map<string, { count: number, resetAt: number }>}
 */
const exportRateLimitStore = new Map();

function checkExportRateLimit(campaignId, apiKeyId) {
  const key = `${campaignId}:${apiKeyId}`;
  const now = Date.now();
  const bucket = exportRateLimitStore.get(key);

  if (!bucket || bucket.resetAt <= now) {
    exportRateLimitStore.set(key, { count: 1, resetAt: now + EXPORT_RATE_LIMIT_WINDOW_MS });
    return { allowed: true, remaining: EXPORT_RATE_LIMIT_MAX - 1 };
  }

  if (bucket.count >= EXPORT_RATE_LIMIT_MAX) {
    return {
      allowed: false,
      remaining: 0,
      resetAt: bucket.resetAt,
    };
  }

  bucket.count += 1;
  return { allowed: true, remaining: EXPORT_RATE_LIMIT_MAX - bucket.count };
}

/**
 * Build CSV from rows, quoting cells per RFC 4180.
 * @param {string[]} columns
 * @param {Record<string, unknown>[]} rows
 * @returns {string}
 */
function buildCsv(columns, rows) {
  const escape = (v) => {
    const s = v === null || v === undefined ? '' : String(v);
    if (s.includes(',') || s.includes('"') || s.includes('\n') || s.includes('\r')) {
      return '"' + s.replace(/"/g, '""') + '"';
    }
    return s;
  };
  const lines = [columns.join(',')];
  for (const row of rows) {
    lines.push(columns.map((c) => escape(row[c])).join(','));
  }
  return lines.join('\n') + '\n';
}

/**
 * @param {{
 *   dal: import('../dal/index.js').Dal,
 *   requireApiKey: import('express').RequestHandler[],
 *   auditLogService: ReturnType<import('../services/auditLogService.js').createAuditLogService>,
 * }} options
 */
export function createCampaignExportRoute({ dal, requireApiKey, auditLogService }) {
  const router = Router();

  router.get('/campaigns/:id/export', requireApiKey, async (req, res) => {
    const { id } = req.params;
    const format = String(req.query.format || 'csv').toLowerCase();
    const fromDate = req.query.from ? String(req.query.from) : null;
    const toDate = req.query.to ? String(req.query.to) : null;

    if (format !== 'csv' && format !== 'json') {
      return res.status(400).json({
        error: 'Invalid format. Use ?format=csv or ?format=json',
        code: 'INVALID_FORMAT',
      });
    }

    const campaign = dal.campaigns.getById(id);
    if (!campaign) {
      return res.status(404).json({ error: 'Campaign not found', code: 'NOT_FOUND' });
    }

    const apiKeyId = req.auth?.keyId ?? req.headers['x-api-key'] ?? 'anon';
    const { allowed, remaining, resetAt } = checkExportRateLimit(id, apiKeyId);
    res.setHeader('X-RateLimit-Limit', String(EXPORT_RATE_LIMIT_MAX));
    res.setHeader('X-RateLimit-Remaining', String(remaining ?? 0));
    if (!allowed) {
      res.setHeader('Retry-After', String(Math.ceil((resetAt - Date.now()) / 1000)));
      return res.status(429).json({
        error: 'Export rate limit exceeded. Max 5 exports per campaign per hour.',
        code: 'EXPORT_RATE_LIMIT_EXCEEDED',
      });
    }

    const participants = dal.campaigns.getExportParticipants(id, { fromDate, toDate });

    auditLogService.log({
      actor: req.auth?.keyId ?? 'api',
      action: 'campaign.export',
      entity: 'campaign',
      entityId: id,
      diff: { format, fromDate, toDate, rowCount: participants.length },
    });

    const filename = `campaign-${id}-export.${format}`;

    if (format === 'csv') {
      const columns = [
        'participantAddress',
        'registeredAt',
        'pointsCredited',
        'pointsClaimed',
        'netPoints',
        'referredBy',
      ];
      const csv = buildCsv(columns, participants);

      res.setHeader('Content-Type', 'text/csv; charset=utf-8');
      res.setHeader('Content-Disposition', `attachment; filename="${filename}"`);

      const readable = Readable.from([csv]);
      await pipeline(readable, res);
    } else {
      const payload = JSON.stringify(
        { campaign: { id: campaign.id, name: campaign.name }, participants },
        null,
        2,
      );

      res.setHeader('Content-Type', 'application/json; charset=utf-8');
      res.setHeader('Content-Disposition', `attachment; filename="${filename}"`);

      const readable = Readable.from([payload]);
      await pipeline(readable, res);
    }
  });

  return router;
}
