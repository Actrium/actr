#!/usr/bin/env node

const puppeteer = require('puppeteer');

const CLIENT_URL = process.env.CLIENT_URL || 'http://localhost:5173';
const MESSAGE = process.env.PYTHON_ECHO_MESSAGE || 'hello-from-actr-web';
const EXPECTED_REPLY_PREFIX = 'echo from python: ';
const TIMEOUT = Number(process.env.PYTHON_ECHO_TIMEOUT_MS || 120000);

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitFor(page, fn, timeout, label, ...args) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await page.evaluate(fn, ...args)) {
      return;
    }
    await sleep(200);
  }
  throw new Error(`Timed out waiting for ${label}`);
}

async function getResultText(page) {
  return page.evaluate(() => document.getElementById('result')?.textContent || '');
}

async function sendEcho(page, message) {
  await page.evaluate((nextMessage) => {
    const input = document.getElementById('msgInput');
    const button = document.getElementById('sendBtn');
    input.value = nextMessage;
    input.dispatchEvent(new Event('input', { bubbles: true }));
    button.click();
  }, message);
}

async function main() {
  const browser = await puppeteer.launch({
    headless: 'new',
    protocolTimeout: 300000,
    args: [
      '--no-sandbox',
      '--disable-setuid-sandbox',
      '--allow-insecure-localhost',
      '--ignore-certificate-errors',
      '--disable-web-security',
      '--disable-features=IsolateOrigins,site-per-process',
    ],
  });

  const page = await browser.newPage();
  const logs = [];
  page.on('console', (msg) => logs.push(`[page] ${msg.text()}`));
  page.on('pageerror', (err) => logs.push(`[pageerror] ${err.message}`));

  try {
    await page.goto(CLIENT_URL, { waitUntil: 'networkidle2', timeout: TIMEOUT });
    await waitFor(
      page,
      () => {
        const status = document.getElementById('status');
        const button = document.getElementById('sendBtn');
        return Boolean(status && status.textContent.includes('✅') && button && !button.disabled);
      },
      TIMEOUT,
      'connected web client'
    );

    await page.evaluate((message) => {
      const input = document.getElementById('msgInput');
      input.value = message;
      input.dispatchEvent(new Event('input', { bubbles: true }));
    }, MESSAGE);

    const waitForReply = (message, timeout) =>
      waitFor(
        page,
        (expectedReply) => {
          const result = document.getElementById('result');
          return Boolean(result && result.textContent.includes(`Reply: "${expectedReply}"`));
        },
        timeout,
        `echo reply for ${message}`,
        `${EXPECTED_REPLY_PREFIX}${message}`
      );

    try {
      await waitForReply(MESSAGE, Math.min(15000, TIMEOUT));
    } catch (_) {
      let lastError = null;
      for (let attempt = 1; attempt <= 3; attempt++) {
        const attemptMessage = `${MESSAGE}-${attempt}`;
        const before = await getResultText(page);
        await waitFor(
          page,
          () => !document.getElementById('sendBtn').disabled,
          TIMEOUT,
          'send button re-enabled'
        );
        await sendEcho(page, attemptMessage);
        try {
          await waitForReply(attemptMessage, 45000);
          lastError = null;
          break;
        } catch (error) {
          lastError = error;
          const after = await getResultText(page);
          if (after === before) {
            await sleep(1000);
          }
        }
      }
      if (lastError) {
        throw lastError;
      }
    }

    console.log(`Python workload web echo PASSED: ${MESSAGE}`);
  } catch (error) {
    console.error(`Python workload web echo FAILED: ${error.message}`);
    const domLogs = await page
      .evaluate(() => document.getElementById('result')?.textContent || '')
      .catch(() => '');
    if (domLogs) {
      console.error('Result DOM log:');
      console.error(domLogs);
    }
    if (logs.length > 0) {
      console.error('Browser console log:');
      for (const line of logs) {
        console.error(line);
      }
    }
    throw error;
  } finally {
    await browser.close().catch(() => {});
  }
}

main().catch(() => {
  process.exit(1);
});
