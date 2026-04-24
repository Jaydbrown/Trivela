import { assertCampaignRepository } from './campaignRepository.js';
import { createSqliteCampaignRepository } from './sqliteCampaignRepository.js';
import { assertAuditLogRepository } from './auditLogRepository.js';
import { createSqliteAuditLogRepository } from './sqliteAuditLogRepository.js';

export function createDal({
  dbPath = ':memory:',
  campaigns = [],
  campaignRepository,
  auditLogRepository,
} = {}) {
  return {
    campaigns: assertCampaignRepository(
      campaignRepository
        ?? createSqliteCampaignRepository({
          dbPath,
          seed: campaigns,
        }),
    ),
    auditLogs: assertAuditLogRepository(
      auditLogRepository ?? createSqliteAuditLogRepository({ dbPath }),
    ),
  };
}
