import { test, expect } from '@playwright/test';

/**
 * Visual regression tests for Storybook components.
 * These tests capture screenshots of each story and compare them against
 * baseline images to detect unintended visual changes.
 * 
 * Run: npm run test:visual
 * Update baselines: npm run test:visual:update
 */

const STORYBOOK_URL = process.env.STORYBOOK_URL || 'http://localhost:6006';

// Stories to test - add new stories here as they're created
const stories = [
  { id: 'components-header--default', name: 'Header - Default' },
  { id: 'components-campaigncard--active', name: 'CampaignCard - Active' },
  { id: 'components-campaigncard--expired', name: 'CampaignCard - Expired' },
  { id: 'components-emptystate--default', name: 'EmptyState - Default' },
  { id: 'components-statusbadge--active', name: 'StatusBadge - Active' },
  { id: 'components-statusbadge--pending', name: 'StatusBadge - Pending' },
  { id: 'components-statusbadge--completed', name: 'StatusBadge - Completed' },
  { id: 'components-transactionstatus--success', name: 'TransactionStatus - Success' },
  { id: 'components-transactionstatus--pending', name: 'TransactionStatus - Pending' },
  { id: 'components-transactionstatus--failed', name: 'TransactionStatus - Failed' },
];

test.describe('Storybook Visual Regression', () => {
  test.beforeEach(async ({ page }) => {
    // Navigate to Storybook and wait for it to load
    await page.goto(STORYBOOK_URL);
    await page.waitForSelector('#storybook-root', { timeout: 10000 });
  });

  for (const story of stories) {
    test(`${story.name} matches snapshot`, async ({ page }) => {
      // Navigate to the specific story
      await page.goto(`${STORYBOOK_URL}/iframe.html?id=${story.id}&viewMode=story`);
      
      // Wait for the story to render
      await page.waitForSelector('#storybook-root > *', { timeout: 5000 });
      
      // Give components time to settle (animations, etc.)
      await page.waitForTimeout(500);
      
      // Take screenshot and compare
      await expect(page).toHaveScreenshot(`${story.id}.png`, {
        fullPage: true,
        animations: 'disabled',
        // Allow small differences due to font rendering across platforms
        maxDiffPixelRatio: 0.02,
      });
    });
  }

  test('all stories load without errors', async ({ page }) => {
    const errors: string[] = [];
    
    page.on('pageerror', (error) => {
      errors.push(`Page error: ${error.message}`);
    });
    
    page.on('console', (msg) => {
      if (msg.type() === 'error') {
        errors.push(`Console error: ${msg.text()}`);
      }
    });

    for (const story of stories) {
      await page.goto(`${STORYBOOK_URL}/iframe.html?id=${story.id}&viewMode=story`);
      await page.waitForSelector('#storybook-root > *', { timeout: 5000 });
      await page.waitForTimeout(500);
    }

    expect(errors).toEqual([]);
  });
});
