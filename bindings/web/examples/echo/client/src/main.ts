/**
 * Echo Client - Actor-RTC Web browser client sample
 *
 * Demonstrates how to use the @actr/web unified Actor API + Local Handler to call a remote Echo service:
 * 1. Create an Actor (shared P2P instance with the Local Handler WASM automatically loaded)
 * 2. The DOM sends requests via callRaw
 * 3. The Local Handler (WASM) discovers the remote Echo Server using ctx.discover()
 * 4. The Local Handler forwards requests via ctx.call_raw() to the remote peer and returns responses
 */

import { createActor, Actor } from '@actr/web';
import { actrConfig, SendEchoActorRef } from './generated';

// ── DOM Elements ──
const statusEl = document.getElementById('status')!;
const sendBtn = document.getElementById('sendBtn') as HTMLButtonElement;
const msgInput = document.getElementById('msgInput') as HTMLInputElement;
const resultEl = document.getElementById('result')!;
const swLogEl = document.getElementById('swLog')!;

let actor: Actor | null = null;
let sendEcho: SendEchoActorRef | null = null;

/**
 * Display logs in the Echo results area
 */
function log(level: 'info' | 'ok' | 'err', message: string): void {
    const time = new Date().toLocaleTimeString('en-US', {
        hour12: false,
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
    });
    const entry = document.createElement('div');
    entry.className = 'entry';
    entry.innerHTML = `<span class="time">${time}</span><span class="${level}">${escapeHtml(message)}</span>`;
    resultEl.appendChild(entry);
    resultEl.scrollTop = resultEl.scrollHeight;

    // Keep log size reasonable
    while (resultEl.children.length > 200) {
        resultEl.removeChild(resultEl.firstChild!);
    }
}

/**
 * Display logs in the SW runtime log panel
 */
function swLog(level: 'info' | 'success' | 'warn' | 'error', message: string): void {
    const time = new Date().toLocaleTimeString('en-US', {
        hour12: false,
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        fractionalSecondDigits: 3,
    });

    const entry = document.createElement('div');
    entry.className = `log-entry ${level}`;
    entry.innerHTML = `<span class="time">${time}</span>${escapeHtml(message)}`;
    swLogEl.appendChild(entry);
    swLogEl.scrollTop = swLogEl.scrollHeight;

    while (swLogEl.children.length > 200) {
        swLogEl.removeChild(swLogEl.firstChild!);
    }
}

function escapeHtml(s: string): string {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/**
 * Initialize the client
 */
async function init(): Promise<void> {
    try {
        statusEl.textContent = 'Connecting...';
        statusEl.className = 'status connecting';

        log('info', '🚀 Initializing Echo Client...');

        // Create an actor via the unified Actor API.
        // The SW-side WASM already contains the Local Handler and automatically
        // handles discover + call_raw forwarding.
        actor = await createActor({
            ...actrConfig,
            serviceWorkerPath: '/actor.sw.js',
            debug: true,
        });

        // Create the type-safe SendEcho ActorRef
        sendEcho = new SendEchoActorRef(actor);

        statusEl.textContent = '✅ Connected';
        statusEl.className = 'status connected';
        sendBtn.disabled = false;

        // Monitor connection state
        actor.on('stateChange', (state) => {
            log('info', `Connection state: ${state}`);
        });

        log('ok', '✅ Client initialized successfully');
        log('info', '⏳ Will automatically send an Echo test message in 5s...');

        // Auto test
        setTimeout(async () => {
            log('info', '🚀 Automatically sending Echo test message...');
            await doSendEcho();
        }, 5000);
    } catch (error) {
        console.error('Failed to initialize client:', error);
        statusEl.textContent = `❌ Connection failed: ${(error as Error).message}`;
        statusEl.className = 'status error';
        log('err', `❌ Initialization failed: ${(error as Error).message}`);
    }
}

/**
 * Send an Echo message
 */
async function doSendEcho(): Promise<void> {
    if (!actor || !sendEcho) {
        log('err', '❌ Client is not initialized');
        return;
    }

    const message = msgInput.value.trim() || `Hello! (${new Date().toLocaleTimeString()})`;

    try {
        sendBtn.disabled = true;
        log('info', `📤 Sending: "${message}"`);

        const response = await sendEcho.sendEcho({ message });

        log('ok', `📥 Reply: "${response.reply}"`);
        log('info', `⏱️ Timestamp: ${new Date(Number(response.timestamp) * 1000).toLocaleString()}`);
    } catch (error) {
        console.error('Echo failed:', error);
        log('err', `❌ Request failed: ${(error as Error).message}`);
    } finally {
        sendBtn.disabled = false;
    }
}

// ── Event Listeners ──
sendBtn.addEventListener('click', doSendEcho);
msgInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !sendBtn.disabled) doSendEcho();
});

// Keep Service Worker alive
setInterval(() => {
    navigator.serviceWorker.controller?.postMessage({ type: 'PING' });
}, 20_000);

// Cleanup on page unload
window.addEventListener('beforeunload', async () => {
    if (actor) await actor.close();
});

/**
 * Listen for runtime logs from the Service Worker
 */
function setupSwLogListener(): void {
    navigator.serviceWorker.addEventListener('message', (event) => {
        const data = event.data;
        if (!data || data.type !== 'sw_log') return;

        const levelMap: Record<string, 'info' | 'success' | 'warn' | 'error'> = {
            info: 'info',
            warn: 'warn',
            error: 'error',
        };
        const uiLevel = levelMap[data.level] || 'info';

        let msg: string = data.message || '';
        msg = msg.replace(/^\s*INFO\s+/, '');
        msg = msg.replace(/^\s*WARN\s+/, '⚠️ ');
        msg = msg.replace(/^\s*ERROR\s+/, '❌ ');

        swLog(uiLevel, msg);
    });
}

// Set up the SW log listener early (before init) to capture startup logs
setupSwLogListener();

// Start
init();
