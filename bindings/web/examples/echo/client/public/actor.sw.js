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

let SW_BROADCAST = null;

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
    SW_BROADCAST = broadcast;

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
 * Verify an .actr package signature and binary hash.
 *
 * This is the Web equivalent of Rust Hyper's verify_package:
 *   1. Read actr.sig (64 bytes Ed25519 signature)
 *   2. Read actr.toml (signed manifest)
 *   3. Verify Ed25519 signature: crypto.subtle.verify('Ed25519', pubkey, sig, manifest)
 *   4. Read binary, compute SHA-256, compare with manifest binary.hash
 *
 * @param {Map<string, Uint8Array>} entries - ZIP entries from parseActrZip
 * @param {string} mfrPubkeyB64 - Base64-encoded Ed25519 MFR public key (32 bytes)
 * @returns {Promise<void>} Resolves if verification passes, throws on failure
 */
async function verifyActrPackage(entries, mfrPubkeyB64) {
    // 1. Read actr.sig
    const sigBytes = entries.get('actr.sig');
    if (!sigBytes || sigBytes.byteLength !== 64) {
        throw new Error('[SW] Package verification failed: actr.sig missing or invalid (expected 64 bytes)');
    }

    // 2. Read actr.toml
    const manifestBytes = entries.get('actr.toml');
    if (!manifestBytes) {
        throw new Error('[SW] Package verification failed: actr.toml missing');
    }

    // 3. Import MFR public key and verify Ed25519 signature
    const pubkeyRaw = Uint8Array.from(atob(mfrPubkeyB64), c => c.charCodeAt(0));
    if (pubkeyRaw.byteLength !== 32) {
        throw new Error('[SW] Package verification failed: MFR public key must be 32 bytes');
    }

    const cryptoKey = await crypto.subtle.importKey(
        'raw',
        pubkeyRaw,
        { name: 'Ed25519' },
        false,
        ['verify']
    );

    const sigValid = await crypto.subtle.verify(
        { name: 'Ed25519' },
        cryptoKey,
        sigBytes,
        manifestBytes
    );

    if (!sigValid) {
        throw new Error('[SW] Package verification failed: Ed25519 signature invalid');
    }

    // 4. Parse manifest to get binary hash
    const manifestText = new TextDecoder().decode(manifestBytes);
    const hashMatch = manifestText.match(/hash\s*=\s*"([0-9a-fA-F]+)"/);
    if (!hashMatch) {
        throw new Error('[SW] Package verification failed: binary hash not found in manifest');
    }
    const expectedHash = hashMatch[1].toLowerCase();

    // 5. Find binary and compute SHA-256
    let binaryBytes = null;
    for (const [name, data] of entries) {
        if (name.startsWith('bin/') && name.endsWith('.wasm')) {
            binaryBytes = data;
            break;
        }
    }
    if (!binaryBytes) {
        throw new Error('[SW] Package verification failed: no WASM binary in package');
    }

    const hashBuffer = await crypto.subtle.digest('SHA-256', binaryBytes);
    const hashArray = new Uint8Array(hashBuffer);
    const actualHash = Array.from(hashArray).map(b => b.toString(16).padStart(2, '0')).join('');

    if (actualHash !== expectedHash) {
        throw new Error(
            '[SW] Package verification failed: binary hash mismatch\n' +
            '  expected: ' + expectedHash + '\n' +
            '  actual:   ' + actualHash
        );
    }
}

/**
 * Load WASM package from a .actr ZIP package.
 *
 * 1. Fetch the .actr package from package_url
 * 2. Parse the ZIP (STORE entries)
 * 3. Verify package signature and binary hash (if mfr_pubkey provided)
 * 4. Find bin/*.wasm and resources/*.js (glue, not actor.sw.js)
 * 5. Eval the JS glue to register wasm_bindgen in global scope
 * 6. Initialize WASM with the binary bytes
 * 7. Call the register function
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

    // ── Package verification (Web verify_package) ──
    // If mfr_pubkey is provided in RUNTIME_CONFIG, verify the .actr package
    // signature and binary hash before loading — equivalent to Rust Hyper's verify_package.
    const mfrPubkey = RUNTIME_CONFIG && RUNTIME_CONFIG.mfr_pubkey;
    if (mfrPubkey) {
        emitSwLog('info', 'actr_verify_start', 'verifying package signature and binary hash');
        try {
            await verifyActrPackage(entries, mfrPubkey);
            emitSwLog('info', 'actr_verify_ok', 'package signature and binary hash verified');
        } catch (verifyError) {
            emitSwLog('error', 'actr_verify_failed', String(verifyError));
            throw verifyError;
        }
    } else {
        emitSwLog('info', 'actr_verify_skip', 'no mfr_pubkey in config, skipping verification');
    }

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
 * Guest Bridge Mode: load runtime WASM and guest WASM separately.
 *
 * When `runtime_wasm_url` is set in RUNTIME_CONFIG, the .actr package
 * contains only the standard guest WASM (built with entry! macro FFI),
 * and the runtime WASM + JS glue are loaded from separate URLs.
 *
 * Protocol:
 *   1. Load runtime WASM + JS glue from runtime_wasm_url
 *   2. Parse .actr package, verify signature, extract guest WASM binary
 *   3. Instantiate guest WASM with host import stubs
 *   4. Call actr_init on the guest
 *   5. Register a JS dispatch callback that bridges AbiFrame → actr_handle → AbiReply
 *   6. Call register_guest_workload(callback) on the runtime
 */
async function loadWithGuestBridge(packageUrl, runtimeWasmUrl) {
    emitSwLog('info', 'guest_bridge_start', { packageUrl, runtimeWasmUrl });

    // ── 1. Load runtime WASM + JS glue ──
    // Derive JS glue URL from WASM URL: "foo_bg.wasm" → "foo.js"
    const jsUrl = runtimeWasmUrl.replace(/_bg\.wasm$/, '.js');
    emitSwLog('info', 'guest_bridge_runtime_js', jsUrl);

    const jsResp = await fetch(jsUrl, { cache: 'no-store' });
    if (!jsResp.ok) {
        throw new Error('[SW] Failed to fetch runtime JS glue: ' + jsResp.status);
    }
    const jsText = await jsResp.text();
    const patchedText = jsText.replace('let wasm_bindgen =', 'self.wasm_bindgen =');
    (0, eval)(patchedText);
    emitSwLog('info', 'guest_bridge_runtime_js_loaded', jsText.length);

    // Init runtime WASM
    await wasm_bindgen({ module_or_path: runtimeWasmUrl });
    wasm_bindgen.init_global();
    emitSwLog('info', 'guest_bridge_runtime_ready', null);

    // ── 2. Load guest WASM from .actr package ──
    const resp = await fetch(packageUrl, { cache: 'no-store' });
    if (!resp.ok) {
        throw new Error('[SW] Failed to fetch .actr package: ' + resp.status);
    }
    const buffer = await resp.arrayBuffer();
    const entries = parseActrZip(buffer);
    emitSwLog('info', 'guest_bridge_actr_entries', Array.from(entries.keys()));

    // Verify package signature
    const mfrPubkey = RUNTIME_CONFIG && RUNTIME_CONFIG.mfr_pubkey;
    if (mfrPubkey && mfrPubkey !== '__MFR_PUBKEY_PLACEHOLDER__') {
        await verifyActrPackage(entries, mfrPubkey);
        emitSwLog('info', 'guest_bridge_verify_ok', null);
    } else {
        emitSwLog('warn', 'guest_bridge_verify_skip', 'No valid MFR pubkey');
    }

    // Extract guest WASM binary
    let guestWasmBytes = null;
    for (const [name, data] of entries) {
        if (name.startsWith('bin/') && name.endsWith('.wasm')) {
            guestWasmBytes = data;
            break;
        }
    }
    if (!guestWasmBytes) {
        throw new Error('[SW] No guest WASM binary found in .actr package');
    }
    emitSwLog('info', 'guest_bridge_guest_wasm', guestWasmBytes.byteLength);

    // ── 3. Instantiate guest WASM with host import stubs ──
    // Standard guest WASMs may import `env.actr_host_invoke` for outbound calls.
    // Echo server doesn't make outbound calls, so stubs suffice.
    const guestImports = {
        env: {
            actr_host_invoke: (_frame_ptr, _frame_len, _reply_ptr, _reply_cap, _reply_len_out) => {
                console.error('[SW Guest] actr_host_invoke called but not implemented in browser');
                return -7; // UNSUPPORTED_OP
            },
            actr_host_self_id: (_buf_ptr, _buf_cap) => -7,
            actr_host_caller_id: (_buf_ptr, _buf_cap) => -7,
            actr_host_request_id: (_buf_ptr, _buf_cap) => -7,
        },
    };

    // Try to instantiate — if no import section exists, empty imports work fine
    let guestModule;
    try {
        guestModule = await WebAssembly.instantiate(guestWasmBytes, guestImports);
    } catch (firstErr) {
        // Fallback: try without imports (guest has no import section)
        try {
            guestModule = await WebAssembly.instantiate(guestWasmBytes, {});
        } catch (secondErr) {
            throw new Error('[SW] Guest WASM instantiation failed: ' + firstErr.message);
        }
    }
    const guest = guestModule.instance.exports;
    emitSwLog('info', 'guest_bridge_guest_instantiated', Object.keys(guest));

    // ── 4. Initialize guest (actr_init) ──
    const actrType = RUNTIME_CONFIG.client_actr_type || '';
    const realmId = RUNTIME_CONFIG.realm_id || 0;
    const initPayload = wasm_bindgen.encode_guest_init_payload(actrType, realmId);

    const initPtr = guest.actr_alloc(initPayload.length);
    if (initPtr === 0) {
        throw new Error('[SW] Guest actr_alloc failed for init payload');
    }
    let guestMem = new Uint8Array(guest.memory.buffer);
    guestMem.set(new Uint8Array(initPayload), initPtr);

    const initResult = guest.actr_init(initPtr, initPayload.length);
    guest.actr_free(initPtr, initPayload.length);

    if (initResult !== 0) {
        throw new Error('[SW] Guest actr_init failed with code: ' + initResult);
    }
    emitSwLog('info', 'guest_bridge_init_ok', null);

    // ── 5. Create dispatch callback ──
    function guestDispatch(abiFrameBytes) {
        try {
            const frameData = new Uint8Array(abiFrameBytes);
            emitSwLog('info', 'guest_dispatch_called', frameData.length);

            // Allocate memory in guest for the request frame
            const reqPtr = guest.actr_alloc(frameData.length);
            if (reqPtr === 0) throw new Error('[SW Guest] actr_alloc failed for request');

            let mem = new Uint8Array(guest.memory.buffer);
            mem.set(frameData, reqPtr);

            // Allocate memory for output pointers (2 × i32 = 8 bytes)
            const outBuf = guest.actr_alloc(8);
            if (outBuf === 0) throw new Error('[SW Guest] actr_alloc failed for output buffer');

            // Call actr_handle
            const result = guest.actr_handle(reqPtr, frameData.length, outBuf, outBuf + 4);
            emitSwLog('info', 'guest_dispatch_actr_handle_result', result);

            // Re-get memory view (buffer may have grown during actr_handle)
            mem = new Uint8Array(guest.memory.buffer);
            const view = new DataView(guest.memory.buffer);

            // Read response pointer and length
            const respPtr = view.getInt32(outBuf, true);
            const respLen = view.getInt32(outBuf + 4, true);
            emitSwLog('info', 'guest_dispatch_resp_ptr_len', { respPtr, respLen });

            // Copy response bytes before freeing
            let response = null;
            if (result === 0 && respPtr !== 0 && respLen > 0) {
                response = new Uint8Array(guest.memory.buffer.slice(respPtr, respPtr + respLen));
            }

            // Free all guest memory
            guest.actr_free(reqPtr, frameData.length);
            guest.actr_free(outBuf, 8);
            if (respPtr !== 0 && respLen > 0) {
                guest.actr_free(respPtr, respLen);
            }

            if (result !== 0) {
                const err = new Error('[SW Guest] actr_handle failed with code: ' + result);
                emitSwLog('error', 'guest_dispatch_error', err.message);
                if (SW_BROADCAST) SW_BROADCAST({ type: 'echo_event', event: 'error', detail: err.message, ts: Date.now() });
                throw err;
            }
            if (!response) {
                const err = new Error('[SW Guest] actr_handle returned empty response');
                emitSwLog('error', 'guest_dispatch_error', err.message);
                if (SW_BROADCAST) SW_BROADCAST({ type: 'echo_event', event: 'error', detail: err.message, ts: Date.now() });
                throw err;
            }

            emitSwLog('info', 'guest_dispatch_ok', response.length);
            if (SW_BROADCAST) {
                SW_BROADCAST({ type: 'echo_event', event: 'request', detail: 'Received via guest_bridge', ts: Date.now() });
                SW_BROADCAST({ type: 'echo_event', event: 'response', detail: 'Sent via guest_bridge', ts: Date.now() });
            }
            return response;
        } catch (e) {
            emitSwLog('error', 'guest_dispatch_exception', String(e));
            if (SW_BROADCAST) {
                SW_BROADCAST({ type: 'echo_event', event: 'error', detail: String(e), ts: Date.now() });
            }
            throw e;
        }
    }

    // ── 6. Register guest workload with runtime ──
    wasm_bindgen.register_guest_workload(guestDispatch);
    emitSwLog('info', 'guest_bridge_ready', 'Guest workload registered via JS bridge');
}

/**
 * Load the WASM package into the Service Worker.
 *
 * This is the Web equivalent of Rust Hyper's load_package_executor:
 *   - Guest bridge: Runtime WASM loaded separately, guest WASM from .actr package
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
    const runtimeWasmUrl = RUNTIME_CONFIG.runtime_wasm_url;

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

        if (runtimeWasmUrl && packageUrl) {
            // Guest bridge mode: load runtime separately, guest from .actr package
            await loadWithGuestBridge(packageUrl, runtimeWasmUrl);
        } else if (packageUrl) {
            // Primary path: load from .actr package (monolithic WASM + JS glue)
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
        console.error('[SW] WASM init failed:', error && error.message ? error.message : String(error),
            'name=' + (error && error.name), 'stack=' + (error && error.stack));
        emitSwLog('error', 'wasm_init_failed', {
            error: error && error.message ? error.message : String(error),
            name: error && error.name ? error.name : undefined,
            stack: error && error.stack ? error.stack : undefined,
            packageUrl: packageUrl || null,
            runtimeWasmUrl: runtimeWasmUrl || null,
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
