#!/usr/bin/env node
/**
 * Diagnostic: Multi-client RPC routing analysis
 * 
 * Tests whether Client 1's echo RPC works after Client 2 connects.
 * Captures ALL SW and DOM messages to identify where the flow breaks.
 */
const puppeteer = require('puppeteer');

const CLIENT_URL = process.env.CLIENT_URL || 'https://localhost:5173';
const TIMEOUT = 60000;

function sleep(ms) { return new Promise(r => setTimeout(r, ms)); }

async function waitForAutoEcho(page, label, timeout = 45000) {
    const start = Date.now();
    console.log(`  [${label}] Waiting for auto-echo (up to ${timeout / 1000}s)...`);
    try {
        await page.waitForFunction(() => {
            const entries = document.querySelectorAll('#result .entry');
            for (const e of entries) {
                if (e.textContent.includes('📥 Reply')) return true;
            }
            return false;
        }, { timeout });
        const ms = Date.now() - start;
        console.log(`  [${label}] Auto-echo OK in ${ms}ms`);
        return { ok: true, ms };
    } catch (e) {
        console.log(`  [${label}] Auto-echo FAILED: ${e.message}`);
        return { ok: false, ms: Date.now() - start, error: e.message };
    }
}

async function manualEcho(page, label, timeout = 15000) {
    const start = Date.now();
    try {
        // Count existing 📥 entries
        const beforeCount = await page.evaluate(() => {
            return [...document.querySelectorAll('#result .entry')]
                .filter(e => e.textContent.includes('📥 Reply')).length;
        });

        // Click send button
        const clicked = await page.evaluate(() => {
            const btn = document.getElementById('sendBtn');
            if (!btn) return 'not_found';
            if (btn.disabled) return 'disabled';
            btn.click();
            return 'clicked';
        });

        if (clicked !== 'clicked') {
            return { ok: false, ms: Date.now() - start, error: `button ${clicked}`, label };
        }

        // Poll for new 📥 entry using page.evaluate in a loop (more reliable than waitForFunction)
        const deadline = Date.now() + timeout;
        while (Date.now() < deadline) {
            await sleep(100);
            const countNow = await page.evaluate(() => {
                return [...document.querySelectorAll('#result .entry')]
                .filter(e => e.textContent.includes('📥 Reply')).length;
            });
            if (countNow > beforeCount) {
                return { ok: true, ms: Date.now() - start, label };
            }
            // Also check for error
            const hasError = await page.evaluate((prevCount) => {
                const errors = [...document.querySelectorAll('#result .entry')]
                    .filter(e => e.textContent.includes('❌'));
                return errors.length > prevCount;
            }, beforeCount);
            if (hasError) {
                return { ok: false, ms: Date.now() - start, error: 'RPC error in UI', label };
            }
        }

        return { ok: false, ms: Date.now() - start, error: `timeout (${timeout}ms)`, label };
    } catch (e) {
        return { ok: false, ms: Date.now() - start, error: e.message, label };
    }
}

async function run() {
    const browser = await puppeteer.launch({
        headless: 'new',
        args: ['--no-sandbox', '--disable-setuid-sandbox', '--ignore-certificate-errors'],
        protocolTimeout: 300_000,
    });

    // Capture ALL console logs with structured data
    function attachLogger(page, name) {
        const logs = [];
        page.on('console', async msg => {
            const text = msg.text();
            // Also try to serialize message args for [object Object]
            let detail = text;
            if (text.includes('[object Object]')) {
                try {
                    const args = msg.args();
                    const parts = [];
                    for (const arg of args) {
                        try {
                            const val = await arg.jsonValue();
                            parts.push(typeof val === 'object' ? JSON.stringify(val).substring(0, 300) : String(val));
                        } catch { parts.push('?'); }
                    }
                    detail = parts.join(' ');
                } catch { }
            }
            logs.push({ t: Date.now(), text: detail });
        });
        return logs;
    }

    // ========== Phase 0: Open Server page to register echo service ==========
    console.log('\n=== Phase 0: Open Echo Server ===');
    const serverPage = await browser.newPage();
    const serverLogs = attachLogger(serverPage, 'Server');
    await serverPage.goto('http://localhost:5174', { waitUntil: 'networkidle2', timeout: TIMEOUT });
    console.log('Server page loaded, waiting for WASM + registration...');

    // Wait for server to register with signaling
    try {
        await serverPage.waitForFunction(() => {
            const entries = document.querySelectorAll('.log-entry, .entry, #swLog div, #status');
            for (const e of entries) {
            if (e.textContent.includes('registered') || e.textContent.includes('Connected')) return true;
            }
            // Also check status element
            const status = document.getElementById('status');
            if (status && status.textContent.includes('Connected')) return true;
            return false;
        }, { timeout: 30000 });
        console.log('Server appears registered');
    } catch {
        console.log('Server registration status unclear, continuing...');
    }
    await sleep(3000); // Extra time for WebSocket stabilization

    // ========== Phase 1: Open Client 1, wait for auto-echo ==========
    console.log('\n=== Phase 1: Open Client 1 ===');
    const page1 = await browser.newPage();
    const logs1 = attachLogger(page1, 'C1');
    await page1.goto(CLIENT_URL, { waitUntil: 'networkidle2', timeout: TIMEOUT });
    console.log('Client 1 loaded');

    const autoEcho1 = await waitForAutoEcho(page1, 'C1');

    // ========== Phase 2: Manual echo from Client 1 (baseline) ==========
    console.log('\n=== Phase 2: Manual echo from Client 1 (baseline) ===');
    const echo1Before = await manualEcho(page1, 'C1-before');
    console.log(`  Result: ${echo1Before.ok ? '✅' : '❌'} ${echo1Before.ms}ms`, echo1Before.error || '');

    // ========== Phase 3: Open Client 2, wait for auto-echo ==========
    console.log('\n=== Phase 3: Open Client 2 ===');
    const page2 = await browser.newPage();
    const logs2 = attachLogger(page2, 'C2');
    await page2.goto(CLIENT_URL, { waitUntil: 'networkidle2', timeout: TIMEOUT });
    console.log('Client 2 loaded');

    const autoEcho2 = await waitForAutoEcho(page2, 'C2');

    // ========== Phase 4: Manual echo from Client 2 ==========
    console.log('\n=== Phase 4: Manual echo from Client 2 ===');
    const echo2 = await manualEcho(page2, 'C2');
    console.log(`  Result: ${echo2.ok ? '✅' : '❌'} ${echo2.ms}ms`, echo2.error || '');

    // ========== Phase 5: Check state ==========
    console.log('\n=== Phase 5: State check ===');
    const sw1 = await page1.evaluate(() => ({
        hasCtrl: !!navigator.serviceWorker.controller,
        state: navigator.serviceWorker.controller?.state,
    }));
    const sw2 = await page2.evaluate(() => ({
        hasCtrl: !!navigator.serviceWorker.controller,
        state: navigator.serviceWorker.controller?.state,
    }));
    console.log('  C1 SW:', JSON.stringify(sw1));
    console.log('  C2 SW:', JSON.stringify(sw2));

    // ========== Phase 6: Manual echo from Client 1 AFTER Client 2 ==========
    console.log('\n=== Phase 6: Client 1 echo AFTER Client 2 ===');
    const markIdx = logs1.length; // mark log position
    const echo1After = await manualEcho(page1, 'C1-after');
    console.log(`  Result: ${echo1After.ok ? '✅' : '❌'} ${echo1After.ms}ms`, echo1After.error || '');

    // Check full UI content of Client 1
    const uiContent = await page1.evaluate(() => {
        const entries = [...document.querySelectorAll('#result .entry')];
        return entries.slice(-15).map(e => e.textContent).join('\n');
    });
    console.log('\n  --- Client 1 UI (last 15 entries) ---');
    console.log(uiContent);

    // Check if the button is still disabled
    const btnState = await page1.evaluate(() => {
        const btn = document.getElementById('sendBtn');
        return btn ? { disabled: btn.disabled, text: btn.textContent } : null;
    });
    console.log('\n  Button state:', JSON.stringify(btnState));

    // Dump relevant logs from this phase  
    console.log('\n  --- ALL Client 1 logs from Phase 6 ---');
    const phase6Logs = logs1.slice(markIdx);
    for (const l of phase6Logs.slice(-50)) {
        console.log(`  ${l.text.substring(0, 300)}`);
    }

    // ========== Phase 7: Client 2 echo again ==========
    console.log('\n=== Phase 7: Client 2 echo again ===');
    const echo2Again = await manualEcho(page2, 'C2-again');
    console.log(`  Result: ${echo2Again.ok ? '✅' : '❌'} ${echo2Again.ms}ms`, echo2Again.error || '');

    // ========== SUMMARY ==========
    console.log('\n=== SUMMARY ===');
    console.log(`C1 auto-echo:    ${autoEcho1.ok ? '✅' : '❌'} ${autoEcho1.ms}ms`);
    console.log(`C1 before C2:    ${echo1Before.ok ? '✅' : '❌'} ${echo1Before.ms}ms`);
    console.log(`C2 auto-echo:    ${autoEcho2.ok ? '✅' : '❌'} ${autoEcho2.ms}ms`);
    console.log(`C2 manual echo:  ${echo2.ok ? '✅' : '❌'} ${echo2.ms}ms`);
    console.log(`C1 AFTER C2:     ${echo1After.ok ? '✅' : '❌'} ${echo1After.ms}ms`);
    console.log(`C2 echo again:   ${echo2Again.ok ? '✅' : '❌'} ${echo2Again.ms}ms`);

    if (echo1Before.ok && !echo1After.ok) {
        console.log('\n❌ Multi-client routing bug confirmed: C1 works alone, fails after C2.');
    } else if (echo1After.ok) {
        console.log('\n✅ Multi-client routing works!');
    } else if (!autoEcho1.ok) {
        console.log('\n⚠️ Single-client echo never works. Dumping ALL C1 logs:');
        for (const l of logs1.slice(-60)) {
            console.log(`  ${l.text.substring(0, 250)}`);
        }
        console.log('\n  Server logs:');
        for (const l of serverLogs.slice(-30)) {
            console.log(`  ${l.text.substring(0, 250)}`);
        }
    } else {
        console.log('\n⚠️ Some issues detected');
    }

    await browser.close();
}

run().catch(e => {
    console.error('Fatal:', e);
    process.exit(1);
});
