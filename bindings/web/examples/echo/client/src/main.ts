/**
 * Echo Client - Actor-RTC Web browser client sample
 *
 * Demonstrates how to use the @actr/web unified Actor API + Local Handler to call a remote Echo service:
 * 1. Create an Actor (shared P2P instance with the Local Handler WASM automatically loaded)
 * 2. The DOM sends requests via callRaw('echo.EchoService.Echo', payload)
 * 3. The Local Handler (WASM) discovers the remote Echo Server using ctx.discover()
 * 4. The Local Handler forwards requests via ctx.call_raw() to the remote peer and returns responses
 */

import { createActor, Actor } from '@actr/web';
import { initConfig, buildActrConfig } from './generated';

// ── Minimal protobuf helpers for EchoRequest / EchoResponse ──
// EchoRequest  { string message = 1; }
// EchoResponse { string reply = 1; uint64 timestamp = 2; }

function encodeEchoRequest(message: string): Uint8Array {
    const msgBytes = new TextEncoder().encode(message);
    // field 1, wire type 2 (length-delimited) = tag 0x0a
    const header = [0x0a, ...encodeVarint(msgBytes.length)];
    const buf = new Uint8Array(header.length + msgBytes.length);
    buf.set(header);
    buf.set(msgBytes, header.length);
    return buf;
}

function decodeEchoResponse(data: Uint8Array): { reply: string; timestamp: number } {
    let reply = '';
    let timestamp = 0;
    let pos = 0;
    while (pos < data.length) {
        const tag = data[pos++];
        const fieldNumber = tag >>> 3;
        const wireType = tag & 0x07;
        if (wireType === 2) {
            // length-delimited
            const [len, bytesRead] = readVarint(data, pos);
            pos += bytesRead;
            if (fieldNumber === 1) {
                reply = new TextDecoder().decode(data.subarray(pos, pos + len));
            }
            pos += len;
        } else if (wireType === 0) {
            // varint
            const [val, bytesRead] = readVarint(data, pos);
            pos += bytesRead;
            if (fieldNumber === 2) {
                timestamp = val;
            }
        } else {
            break; // unknown wire type
        }
    }
    return { reply, timestamp };
}

function encodeVarint(value: number): number[] {
    const bytes: number[] = [];
    while (value > 0x7f) {
        bytes.push((value & 0x7f) | 0x80);
        value >>>= 7;
    }
    bytes.push(value & 0x7f);
    return bytes;
}

function readVarint(data: Uint8Array, offset: number): [number, number] {
    let result = 0;
    let shift = 0;
    let bytesRead = 0;
    while (offset < data.length) {
        const b = data[offset++];
        bytesRead++;
        result |= (b & 0x7f) << shift;
        if ((b & 0x80) === 0) break;
        shift += 7;
    }
    return [result >>> 0, bytesRead];
}

// ── DOM Elements ──
const statusEl = document.getElementById('status')!;
const sendBtn = document.getElementById('sendBtn') as HTMLButtonElement;
const msgInput = document.getElementById('msgInput') as HTMLInputElement;
const resultEl = document.getElementById('result')!;
const swLogEl = document.getElementById('swLog')!;

let actor: Actor | null = null;

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
        statusEl.textContent = 'Loading config...';
        statusEl.className = 'status connecting';

        log('info', '📡 Loading runtime config...');

        // Load runtime config from /actr-runtime-config.json
        await initConfig();
        const actrConfig = buildActrConfig();

        statusEl.textContent = 'Connecting...';
        log('info', '🚀 Initializing Echo Client...');

        // Create an actor via the unified Actor API.
        // The SW-side WASM already contains the Local Handler and automatically
        // handles discover + call_raw forwarding.
        actor = await createActor({
            ...actrConfig,
            serviceWorkerPath: '/actor.sw.js',
            debug: true,
        });

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
    if (!actor) {
        log('err', '❌ Client is not initialized');
        return;
    }

    const message = msgInput.value.trim() || `Hello! (${new Date().toLocaleTimeString()})`;

    try {
        sendBtn.disabled = true;
        log('info', `📤 Sending: "${message}"`);

        // Encode EchoRequest and call the local handler via callRaw
        const payload = encodeEchoRequest(message);
        const responseData = await actor.callRaw('echo.EchoService.Echo', payload);
        const response = decodeEchoResponse(responseData);

        log('ok', `📥 Reply: "${response.reply}"`);
        log('info', `⏱️ Timestamp: ${new Date(response.timestamp * 1000).toLocaleString()}`);
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
