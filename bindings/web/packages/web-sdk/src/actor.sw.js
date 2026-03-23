/* Actor-RTC Generic Service Worker entry.
 *
 * This SW loads a WASM package from a signed .actr ZIP package.
 * The .actr package is fetched from the URL specified in runtimeConfig.package_url.
 *
 * Package loading fields in runtimeConfig:
 *   package_url    - URL of the .actr package (e.g. "/packages/echo-server.actr")
 *   register_fn    - name of the wasm_bindgen function to call after init
 *                    (e.g. "register_echo_service")
 *
 * Legacy fallback (when package_url is not set):
 *   package_js     - filename of the wasm-bindgen JS glue (e.g. "echo_server.js")
 *   package_wasm   - filename of the WASM binary (e.g. "echo_server_bg.wasm")
 *
 * .actr ZIP format (all entries use STORE / no compression):
 *   actr.toml                  - package manifest
 *   actr.sig                   - Ed25519 signature (64 bytes)
 *   bin/actor.wasm             - WASM binary
 *   resources/glue.js          - wasm-bindgen JS glue
 *
 * This is the Web equivalent of Rust Hyper's load_package_executor:
 * it loads a WASM workload package into the SW runtime on demand.
 */

/* global wasm_bindgen */

// ── Console interception: forward WASM logs to main page ──
(function () {
    const _origInfo = console.info;
    const _origWarn = console.warn;
    const _origError = console.error;
    const _origLog = console.log;

    function extractMessage(args) {
        return Array.from(args)
            .filter(a => typeof a === 'string' && !/^\s*(color|background|font-weight|padding)\s*:/.test(a))
            .join(' ')
            .replace(/%c/g, '')
            .trim();
    }

    function broadcast(data) {
        self.clients.matchAll({ type: 'window' }).then(clients => {
            for (const client of clients) {
                client.postMessage(data);
            }
        }).catch(() => { /* ignore */ });
    }

    console.info = function (...args) {
        _origInfo.apply(console, args);
        const msg = extractMessage(args);

        // Echo service events (detect log markers from WASM)
        if (msg.includes('📨') && msg.includes('Echo request')) {
            const m = msg.match(/message='([^']*)'/);
            broadcast({ type: 'echo_event', event: 'request', detail: m ? m[1] : '', ts: Date.now() });
        } else if (msg.includes('📤') && msg.includes('Echo response')) {
            const m = msg.match(/reply='([^']*)'/);
            broadcast({ type: 'echo_event', event: 'response', detail: m ? m[1] : '', ts: Date.now() });
        }

        if (msg.includes('[SW]') || msg.includes('EchoService') || msg.includes('Echo')
            || msg.includes('Registering') || msg.includes('SendEcho')
            || msg.includes('Scheduler') || msg.includes('Dispatcher')
            || msg.includes('HostGate') || msg.includes('PeerGate')) {
            broadcast({ type: 'sw_log', level: 'info', message: msg, ts: Date.now() });
        }
    };

    console.warn = function (...args) {
        _origWarn.apply(console, args);
        const msg = extractMessage(args);
        if (msg.length > 0) {
            broadcast({ type: 'sw_log', level: 'warn', message: msg, ts: Date.now() });
        }
    };

    console.error = function (...args) {
        _origError.apply(console, args);
        const msg = extractMessage(args);
        broadcast({ type: 'sw_log', level: 'error', message: msg, ts: Date.now() });
        if (msg.includes('Echo') || msg.includes('handle_request') || msg.includes('service')) {
            broadcast({ type: 'echo_event', event: 'error', detail: msg, ts: Date.now() });
        }
    };

    console.log = function (...args) {
        _origLog.apply(console, args);
        const msg = extractMessage(args);
        if (msg.includes('[EchoService]') || msg.includes('[SW]') || msg.includes('[SendEcho]')
            || msg.includes('[WebRTC]')) {
            broadcast({ type: 'sw_log', level: 'info', message: msg, ts: Date.now() });
        }
    };
})();

/** @type {import('@actr/web').SwRuntimeConfig | null} */
let RUNTIME_CONFIG = null;

let wasmReady = false;
let wsProbeDone = false;

const clientPorts = new Map();
const browserToSwClient = new Map();

async function cleanupStaleClients() {
    if (!wasmReady) return;
    try {
        const activeWindows = await self.clients.matchAll({ type: 'window' });
        const activeIds = new Set(activeWindows.map(c => c.id));
        for (const [browserId, swClientId] of browserToSwClient.entries()) {
            if (!activeIds.has(browserId)) {
                console.log('[SW] Cleaning up stale client:', swClientId, '(browser:', browserId, ')');
                browserToSwClient.delete(browserId);
                clientPorts.delete(swClientId);
                try {
                    await wasm_bindgen.unregister_client(swClientId);
                } catch (e) {
                    console.warn('[SW] unregister_client error for', swClientId, ':', e);
                }
            }
        }
    } catch (e) {
        console.warn('[SW] cleanupStaleClients error:', e);
    }
}

function emitSwLog(level, message, detail) {
    for (const port of clientPorts.values()) {
        try {
            port.postMessage({
                type: 'webrtc_event',
                payload: {
                    eventType: 'sw_log',
                    data: { level, message, detail },
                },
            });
        } catch (_) { /* port may be closed */ }
    }
}

// ── .actr ZIP parser (STORE-only, no compression) ──

/**
 * Parse a ZIP file that uses STORE method (no compression).
 * .actr packages always use CompressionMethod::Stored.
 *
 * Iterates Local File Headers sequentially. Each header:
 *   4 bytes  signature  (0x04034b50 = PK\x03\x04)
 *   2 bytes  version needed
 *   2 bytes  flags
 *   2 bytes  compression method (0 = STORE)
 *   2 bytes  mod time
 *   2 bytes  mod date
 *   4 bytes  CRC-32
 *   4 bytes  compressed size
 *   4 bytes  uncompressed size
 *   2 bytes  filename length
 *   2 bytes  extra field length
 *   N bytes  filename
 *   M bytes  extra field
 *   S bytes  data (compressed size == uncompressed size for STORE)
 *
 * @param {ArrayBuffer} buffer - The ZIP file bytes
 * @returns {Map<string, Uint8Array>} filename → file contents
 */
function parseActrZip(buffer) {
    const view = new DataView(buffer);
    const bytes = new Uint8Array(buffer);
    const entries = new Map();
    let offset = 0;

    while (offset + 30 <= buffer.byteLength) {
        const sig = view.getUint32(offset, true);
        // Local File Header signature
        if (sig !== 0x04034b50) break;

        const compressedSize = view.getUint32(offset + 18, true);
        const uncompressedSize = view.getUint32(offset + 22, true);
        const filenameLen = view.getUint16(offset + 26, true);
        const extraLen = view.getUint16(offset + 28, true);

        const filenameBytes = bytes.subarray(offset + 30, offset + 30 + filenameLen);
        const filename = new TextDecoder().decode(filenameBytes);

        const dataStart = offset + 30 + filenameLen + extraLen;
        const dataEnd = dataStart + compressedSize;

        if (dataEnd > buffer.byteLength) {
            console.warn('[SW] ZIP entry truncated:', filename);
            break;
        }

        // Store a copy of the data (not a view, so the buffer can be GC'd)
        entries.set(filename, bytes.slice(dataStart, dataEnd));

        offset = dataEnd;
    }

    return entries;
}

/**
 * Load WASM package from a .actr ZIP package.
 *
 * 1. Fetch the .actr package from package_url
 * 2. Parse the ZIP (STORE entries)
 * 3. Find bin/*.wasm and resources/*.js (glue, not actor.sw.js)
 * 4. Eval the JS glue to register wasm_bindgen in global scope
 * 5. Initialize WASM with the binary bytes
 * 6. Call the register function
 */
async function loadFromActrPackage(packageUrl, registerFn) {
    emitSwLog('info', 'actr_package_fetch', packageUrl);

    const resp = await fetch(packageUrl, { cache: 'no-store' });
    if (!resp.ok) {
        throw new Error('[SW] Failed to fetch .actr package: ' + resp.status + ' ' + resp.statusText);
    }

    const buffer = await resp.arrayBuffer();
    emitSwLog('info', 'actr_package_size', buffer.byteLength);

    const entries = parseActrZip(buffer);
    emitSwLog('info', 'actr_zip_entries', Array.from(entries.keys()));

    // Find the WASM binary (bin/*.wasm or bin/actor.wasm)
    let wasmBytes = null;
    let wasmName = null;
    for (const [name, data] of entries) {
        if (name.startsWith('bin/') && name.endsWith('.wasm')) {
            wasmBytes = data;
            wasmName = name;
            break;
        }
    }
    if (!wasmBytes) {
        throw new Error('[SW] No WASM binary found in .actr package');
    }
    emitSwLog('info', 'actr_wasm_found', { name: wasmName, size: wasmBytes.byteLength });

    // Find the JS glue (resources/*.js but NOT actor.sw.js)
    let glueText = null;
    let glueName = null;
    for (const [name, data] of entries) {
        if (name.startsWith('resources/') && name.endsWith('.js') && !name.endsWith('actor.sw.js')) {
            glueText = new TextDecoder().decode(data);
            glueName = name;
            break;
        }
    }
    if (!glueText) {
        throw new Error('[SW] No JS glue found in .actr package');
    }
    emitSwLog('info', 'actr_glue_found', { name: glueName, size: glueText.length });

    // Eval the JS glue — patch 'let wasm_bindgen =' to 'self.wasm_bindgen ='
    try {
        const patchedText = glueText.replace('let wasm_bindgen =', 'self.wasm_bindgen =');
        (0, eval)(patchedText);
        emitSwLog('info', 'actr_eval_loaded', patchedText.length);
    } catch (error) {
        emitSwLog('error', 'actr_eval_failed', String(error));
        throw error;
    }

    // Initialize WASM from raw bytes
    // wasm_bindgen accepts { module_or_path: ArrayBuffer|Uint8Array }
    emitSwLog('info', 'actr_wasm_init', wasmBytes.byteLength);
    await wasm_bindgen({ module_or_path: wasmBytes });
    emitSwLog('info', 'actr_wasm_ready', null);

    wasm_bindgen.init_global();

    // Call the workload registration function
    if (registerFn && typeof wasm_bindgen[registerFn] === 'function') {
        wasm_bindgen[registerFn]();
        emitSwLog('info', 'workload_registered', registerFn);
    } else if (registerFn) {
        console.warn('[SW] register function not found:', registerFn);
    }
}

/**
 * Legacy: Load WASM from direct JS/WASM URLs (development fallback).
 */
async function loadFromDirectUrls(jsFile, wasmFile, registerFn) {
    const runtimeUrl = new URL(jsFile, self.location).toString();
    const wasmUrl = new URL(wasmFile, self.location).toString();

    emitSwLog('info', 'legacy_runtime_url', runtimeUrl);

    const runtimeRes = await fetch(runtimeUrl, { cache: 'no-store' });
    emitSwLog('info', 'legacy_runtime_fetch', {
        url: runtimeUrl,
        status: runtimeRes.status,
    });

    const wasmRes = await fetch(wasmUrl, { cache: 'no-store' });
    emitSwLog('info', 'legacy_wasm_fetch', {
        url: wasmUrl,
        status: wasmRes.status,
    });

    const runtimeText = await runtimeRes.text();
    const patchedText = runtimeText.replace('let wasm_bindgen =', 'self.wasm_bindgen =');
    (0, eval)(patchedText);

    await wasm_bindgen({ module_or_path: wasmUrl });
    wasm_bindgen.init_global();

    if (registerFn && typeof wasm_bindgen[registerFn] === 'function') {
        wasm_bindgen[registerFn]();
        emitSwLog('info', 'workload_registered', registerFn);
    } else if (registerFn) {
        console.warn('[SW] register function not found:', registerFn);
    }
}

/**
 * Load the WASM package into the Service Worker.
 *
 * This is the Web equivalent of Rust Hyper's load_package_executor:
 *   - Primary: Fetch a .actr package (signed ZIP), extract and load WASM + JS glue
 *   - Legacy fallback: Fetch separate JS glue + WASM files
 *
 * The package info comes from RUNTIME_CONFIG (set via DOM_PORT_INIT).
 */
async function ensureWasmReady() {
    if (wasmReady) return;

    if (!RUNTIME_CONFIG) {
        throw new Error('[SW] Cannot load WASM: RUNTIME_CONFIG not yet received');
    }

    const packageUrl = RUNTIME_CONFIG.package_url;
    const registerFn = RUNTIME_CONFIG.register_fn;

    try {
        // WebSocket probe (once)
        if (!wsProbeDone) {
            wsProbeDone = true;
            try {
                emitSwLog('info', 'ws_probe_start', RUNTIME_CONFIG.signaling_url);
                const probe = new WebSocket(RUNTIME_CONFIG.signaling_url);
                probe.binaryType = 'arraybuffer';
                probe.onopen = () => {
                    emitSwLog('info', 'ws_probe_open', null);
                    probe.close();
                };
                probe.onerror = () => {
                    emitSwLog('error', 'ws_probe_error', null);
                };
                probe.onclose = (event) => {
                    emitSwLog('info', 'ws_probe_close', {
                        code: event.code,
                        reason: event.reason,
                        wasClean: event.wasClean,
                    });
                };
            } catch (error) {
                emitSwLog('error', 'ws_probe_throw', String(error));
            }
        }

        if (packageUrl) {
            // Primary path: load from .actr package
            await loadFromActrPackage(packageUrl, registerFn);
        } else {
            // Legacy fallback: load from separate URLs
            const jsFile = RUNTIME_CONFIG.package_js;
            const wasmFile = RUNTIME_CONFIG.package_wasm;
            if (!jsFile || !wasmFile) {
                throw new Error('[SW] Missing package_url (or package_js/package_wasm) in runtimeConfig');
            }
            await loadFromDirectUrls(jsFile, wasmFile, registerFn);
        }

        wasmReady = true;
        emitSwLog('info', 'wasm_ready', null);
    } catch (error) {
        console.error('[SW] WASM init failed:', error);
        emitSwLog('error', 'wasm_init_failed', {
            error: String(error),
            name: error && error.name ? error.name : undefined,
            stack: error && error.stack ? error.stack : undefined,
            packageUrl: packageUrl || null,
        });
        throw error;
    }
}

self.addEventListener('install', (event) => {
    console.log('[SW] installing...');
    event.waitUntil(self.skipWaiting());
});

self.addEventListener('activate', (event) => {
    console.log('[SW] activated');
    event.waitUntil(self.clients.claim());
});

self.addEventListener('message', (event) => {
    if (event.data && event.data.type === 'PING') {
        if (event.source && event.source.postMessage) {
            event.source.postMessage({ type: 'PONG' });
        }
        return;
    }

    if (!event.data || event.data.type !== 'DOM_PORT_INIT') {
        return;
    }

    const port = event.data.port;
    const clientId = event.data.clientId;
    if (!port || !clientId) return;

    if (event.data.runtimeConfig && !RUNTIME_CONFIG) {
        RUNTIME_CONFIG = event.data.runtimeConfig;
    }

    clientPorts.set(clientId, port);
    const browserId = event.source && event.source.id;
    if (browserId) {
        browserToSwClient.set(browserId, clientId);
    }

    cleanupStaleClients();

    console.log('[SW] port initialized for client:', clientId, 'total:', clientPorts.size);

    if (event.source && event.source.postMessage) {
        event.source.postMessage({ type: 'sw_ack', message: 'port_ready' });
    }

    emitSwLog('info', 'sw_env', {
        clientId,
        hasWindow: typeof window !== 'undefined',
        hasSetTimeout: typeof setTimeout,
        location: self.location ? self.location.href : null,
        totalClients: clientPorts.size,
    });

    port.onmessage = async (portEvent) => {
        try {
            await ensureWasmReady();
        } catch (error) {
            console.error('[SW] WASM not ready:', error);
            return;
        }

        const message = portEvent.data;
        if (!message || !message.type) return;

        switch (message.type) {
            case 'control':
                try {
                    await wasm_bindgen.handle_dom_control(clientId, message.payload);
                } catch (error) {
                    console.error('[SW] handle_dom_control failed:', error);
                    emitSwLog('error', 'handle_dom_control_failed', String(error));
                }
                break;

            case 'webrtc_event':
                try {
                    await wasm_bindgen.handle_dom_webrtc_event(clientId, message.payload);
                } catch (error) {
                    console.error('[SW] handle_dom_webrtc_event failed:', error);
                    emitSwLog('error', 'handle_dom_webrtc_event_failed', String(error));
                }
                break;

            case 'fast_path_data':
                try {
                    await wasm_bindgen.handle_dom_fast_path(clientId, message.payload);
                } catch (error) {
                    console.error('[SW] handle_dom_fast_path failed:', error);
                    emitSwLog('error', 'handle_dom_fast_path_failed', String(error));
                }
                break;

            case 'register_datachannel_port':
                try {
                    const dcPort = message.payload.port;
                    const dcPeerId = message.payload.peerId;
                    if (dcPort && dcPeerId) {
                        await wasm_bindgen.register_datachannel_port(clientId, dcPeerId, dcPort);
                    } else {
                        console.warn('[SW] register_datachannel_port: missing port or peerId');
                    }
                } catch (error) {
                    console.error('[SW] register_datachannel_port failed:', error);
                    emitSwLog('error', 'register_datachannel_port_failed', String(error));
                }
                break;

            default:
                console.log('[SW] unknown message type:', message.type);
                break;
        }
    };

    port.start();

    ensureWasmReady().then(async () => {
        try {
            if (!RUNTIME_CONFIG) {
                console.error('[SW] RUNTIME_CONFIG not received from main thread');
                return;
            }
            await wasm_bindgen.register_client(clientId, RUNTIME_CONFIG, port);
            console.log('[SW] Client registered:', clientId);
            emitSwLog('info', 'client_registered', { clientId });
        } catch (error) {
            console.error('[SW] register_client failed:', error);
            emitSwLog('error', 'register_client_failed', { clientId, error: String(error) });
        }
    });
});
