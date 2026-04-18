/**
 * Echo Server - Actor-RTC Web browser-hosted server sample
 *
 * Demonstrates how to host a browser-side Echo service with the @actr/web API:
 * 1. Use createActor to build the Actor (initializes the SW bridge + WebRTC)
 * 2. The EchoService workload inside the WASM Service Worker handles the RPC surface
 * 3. The SW broadcasts console events back to the page through clients.postMessage
 *
 * Metrics pipeline:
 * ┌───────────────────────────────────────────────┐
 * │  1. echo_service.rs in WASM logs via log::info! │
 * │  2. wasm_logger forwards to console.info()     │
 * │  3. actor.sw.js intercepts console.info and tags 📨/📤 │
 * │  4. Broadcast via self.clients.postMessage to the main page │
 * │  5. navigator.serviceWorker.onmessage updates the UI      │
 * └───────────────────────────────────────────────┘
 */

import { createActor, type Actor } from '@actr/web';
import { initConfig, actrType, system, buildActrConfig } from './generated';

// ── DOM Elements ──
const statusEl = document.getElementById('status')!;
const serviceNameEl = document.getElementById('serviceName')!;
const signalingUrlEl = document.getElementById('signalingUrl')!;
const realmIdEl = document.getElementById('realmId')!;
const actrTypeEl = document.getElementById('actrType')!;
const requestCountEl = document.getElementById('requestCount')!;
const successCountEl = document.getElementById('successCount')!;
const errorCountEl = document.getElementById('errorCount')!;
const logEl = document.getElementById('log')!;

let actor: Actor | null = null;

// ── Real metrics counters (updated by SW events) ──
let requestCount = 0;
let successCount = 0;
let errorCount = 0;

/**
 * Log to the UI
 */
function log(level: 'info' | 'success' | 'warn' | 'error', message: string): void {
    const time = new Date().toLocaleTimeString('zh-CN', {
        hour12: false,
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        fractionalSecondDigits: 3,
    });

    const entry = document.createElement('div');
    entry.className = `log-entry ${level}`;
    entry.innerHTML = `<span class="time">${time}</span>${escapeHtml(message)}`;
    logEl.appendChild(entry);
    logEl.scrollTop = logEl.scrollHeight;

    // Keep log size reasonable
    while (logEl.children.length > 200) {
        logEl.removeChild(logEl.firstChild!);
    }
}

function escapeHtml(s: string): string {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/**
 * Update the stats display
 */
function updateStatsUI(): void {
    requestCountEl.textContent = String(requestCount);
    successCountEl.textContent = String(successCount);
    errorCountEl.textContent = String(errorCount);
}

/**
 * Update the server info display
 */
function updateServerInfo(): void {
    serviceNameEl.textContent = `echo.${actrType.name}`;
    signalingUrlEl.textContent = system.signaling.url;
    realmIdEl.textContent = String(system.deployment.realm_id);
    actrTypeEl.textContent = actrType.fullType;
}

/**
 * Listen for real events from the Service Worker
 *
 * actor.sw.js console interception broadcasts echo_event and sw_log messages via self.clients.postMessage
 */
function setupSwEventListener(): void {
    navigator.serviceWorker.addEventListener('message', (event) => {
        const data = event.data;
        if (!data || !data.type) return;

        switch (data.type) {
            case 'echo_event':
                handleEchoEvent(data);
                break;
            case 'sw_log':
                handleSwLog(data);
                break;
            // Ignore PONG, sw_ack, etc.
        }
    });
}

/**
 * Handle real Echo RPC events (via log::info! from WASM echo_service.rs)
 */
function handleEchoEvent(data: {
    event: 'request' | 'response' | 'error';
    detail: string;
    ts: number;
}): void {
    switch (data.event) {
        case 'request':
            requestCount++;
            log('info', `📨 Received Echo request: "${data.detail}"`);
            break;
        case 'response':
            successCount++;
            log('success', `✅ Sent Echo response: "${data.detail}"`);
            break;
        case 'error':
            errorCount++;
            log('error', `❌ Handling error: ${data.detail}`);
            break;
    }
    updateStatsUI();
}

/**
 * Process SW runtime logs (log::info/warn/error from the WASM runtime)
 */
function handleSwLog(data: {
    level: 'info' | 'warn' | 'error';
    message: string;
    ts: number;
}): void {
    const levelMap: Record<string, 'info' | 'success' | 'warn' | 'error'> = {
        info: 'info',
        warn: 'warn',
        error: 'error',
    };
    const uiLevel = levelMap[data.level] || 'info';

    // Shorten noisy log prefixes for cleaner display
    let msg = data.message;
    msg = msg.replace(/^\s*INFO\s+/, '');
    msg = msg.replace(/^\s*WARN\s+/, '⚠️ ');
    msg = msg.replace(/^\s*ERROR\s+/, '❌ ');

    log(uiLevel, msg);
}

/**
 * Initialize and start the server
 */
async function startServer(): Promise<void> {
    try {
        log('info', '🚀 Starting Echo Server...');
        log('info', '� Loading runtime config...');

        // Load runtime config from /actr-runtime-config.json
        await initConfig();
        const actrConfig = buildActrConfig();

        log('info', '�📦 WASM will be loaded by the Service Worker (actor.sw.js)');

        // Update the UI with server information
        updateServerInfo();

        // Set up SW event listeners before createActor to capture initialization logs
        setupSwEventListener();

        // Create the Actor — this initializes the SW bridge and WebRTC
        // The Rust EchoService workload inside the WASM SW handles the RPC logic
        actor = await createActor({
            ...actrConfig,
            serviceWorkerPath: '/actor.sw.js',
        });

        // Update the status display
        statusEl.innerHTML = '<span>✅</span><span>Server is running</span>';
        statusEl.className = 'status ready';

        log('success', '✅ Echo Server started successfully!');
        log('info', '📡 Registered service: echo.EchoService');
        log('info', '⏳ Waiting for clients to connect...');

        console.log('Echo Server started successfully');
    } catch (error) {
        console.error('Failed to start server:', error);
        log('error', `❌ Startup failed: ${(error as Error).message}`);

        statusEl.innerHTML = `<span>❌</span><span>Startup failed: ${(error as Error).message}</span>`;
        statusEl.className = 'status error';
    }
}

// Close the Actor on page unload
window.addEventListener('beforeunload', async () => {
    if (actor) {
        await actor.close();
    }
});

// Keep Service Worker alive by sending periodic pings.
// SW will be terminated by the browser if idle for ~30s.
let keepAliveTimer: ReturnType<typeof setInterval> | null = null;
function startKeepAlive(): void {
    if (keepAliveTimer) return;
    keepAliveTimer = setInterval(() => {
        navigator.serviceWorker.controller?.postMessage({ type: 'PING' });
    }, 20_000); // every 20s
}

// Start the server
startServer().then(() => {
    startKeepAlive();
});
