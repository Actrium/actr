/* Actor-RTC Generic Service Worker entry.
 *
 * Loads the SW runtime WASM from `runtime_wasm_url`, then fetches the
 * signed `.actr` workload package from `package_url`, verifies it via the
 * Rust runtime's `verify_and_extract_actr_package`, and instantiates the
 * extracted guest WASM with host import stubs (guest-bridge).
 *
 * Runtime and workload are always separate artifacts — the SW never loads
 * an unverified monolithic bundle, and verification is mandatory (no
 * skip-verify path).
 *
 * Required `runtimeConfig` fields:
 *   package_url       - URL of the `.actr` workload package
 *   runtime_wasm_url  - URL of the SW runtime WASM (wasm-pack output)
 *   trust             - array of `TrustAnchor` entries; must include a
 *                       `kind="static"` anchor with `pubkey_b64`
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

/**
 * Load runtime WASM, verify + extract the `.actr` workload in Rust, then
 * instantiate the guest WASM behind host import stubs.
 *
 * Hyper (SW runtime) and workload (guest) are separate artifacts. The
 * runtime is loaded from `runtime_wasm_url`; the workload from the signed
 * `.actr` at `package_url`. The runtime's `verify_and_extract_actr_package`
 * is the sole verifier — it hard-fails if no static trust anchor is
 * configured, mirroring native Hyper's mandatory-verify contract.
 *
 * Flow:
 *   1. Load runtime WASM + wasm-bindgen JS glue from `runtime_wasm_url`.
 *   2. Fetch the `.actr` bytes.
 *   3. Call `verify_and_extract_actr_package(bytes, trust_json)` in Rust
 *      to verify Ed25519 + binary hash and extract the guest WASM binary.
 *   4. Instantiate the guest with host import stubs (JSPI path if
 *      available, sync fallback otherwise).
 *   5. Run `actr_init` on the guest.
 *   6. Register a JS dispatch callback bridging AbiFrame → actr_handle →
 *      AbiReply with the runtime via `register_guest_workload`.
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

    // ── 2. Fetch + Rust-side verify + extract guest WASM ──
    const resp = await fetch(packageUrl, { cache: 'no-store' });
    if (!resp.ok) {
        throw new Error('[SW] Failed to fetch .actr package: ' + resp.status);
    }
    const buffer = await resp.arrayBuffer();
    emitSwLog('info', 'guest_bridge_actr_size', buffer.byteLength);

    const trustJson = JSON.stringify(
        (RUNTIME_CONFIG && Array.isArray(RUNTIME_CONFIG.trust)) ? RUNTIME_CONFIG.trust : []
    );
    let extracted;
    try {
        extracted = wasm_bindgen.verify_and_extract_actr_package(
            new Uint8Array(buffer),
            trustJson,
        );
        emitSwLog('info', 'guest_bridge_verify_ok', null);
    } catch (verifyError) {
        emitSwLog('error', 'guest_bridge_verify_failed', String(verifyError));
        throw verifyError;
    }

    const guestWasmBytes = extracted.binary;
    emitSwLog('info', 'guest_bridge_guest_wasm', guestWasmBytes.byteLength);

    // ── 3. Detect JSPI support ──
    // JSPI (JavaScript Promise Integration) allows WASM imports to suspend
    // execution when they return a Promise, without needing asyncify.
    // Available in Chrome 129+ (Sep 2024).
    const hasJSPI = typeof WebAssembly.Suspending === 'function'
        && typeof WebAssembly.promising === 'function';
    emitSwLog('info', 'guest_bridge_jspi', hasJSPI);

    // ── 4. Instantiate guest WASM with host imports ──
    let guest;
    let promisingActrHandle = null;  // JSPI-wrapped actr_handle (returns Promise)

    if (hasJSPI) {
        // ── JSPI path: async actr_host_invoke, promising actr_handle ──
        // The guest's actr_host_invoke import is wrapped with WebAssembly.Suspending
        // so the WASM execution suspends while the host performs async operations
        // (discover, call_raw). No wasm-opt --asyncify needed.
        const asyncHostInvoke = async function (frame_ptr, frame_len, reply_ptr, reply_cap, reply_len_out) {
            try {
                // Read ABI frame from guest memory
                const mem = new Uint8Array(guest.memory.buffer);
                const frameData = mem.slice(frame_ptr, frame_ptr + frame_len);

                // Call runtime's guest_host_invoke_async (returns a Promise)
                const replyBytes = await wasm_bindgen.guest_host_invoke_async(frameData);

                if (replyBytes.length > reply_cap) {
                    // BUFFER_TOO_SMALL: write needed size and return error
                    const view = new DataView(guest.memory.buffer);
                    view.setInt32(reply_len_out, replyBytes.length, true);
                    return -6; // BUFFER_TOO_SMALL
                }

                // Write reply into guest memory
                const writeMem = new Uint8Array(guest.memory.buffer);
                writeMem.set(replyBytes, reply_ptr);
                const writeView = new DataView(guest.memory.buffer);
                writeView.setInt32(reply_len_out, replyBytes.length, true);

                return 0; // SUCCESS
            } catch (e) {
                emitSwLog('error', 'guest_host_invoke_error', String(e));
                return -1; // GENERIC_ERROR
            }
        };

        const guestImports = {
            env: {
                actr_host_invoke: new WebAssembly.Suspending(asyncHostInvoke),
                actr_host_self_id: (_buf_ptr, _buf_cap) => -7,
                actr_host_caller_id: (_buf_ptr, _buf_cap) => -7,
                actr_host_request_id: (_buf_ptr, _buf_cap) => -7,
            },
        };

        let guestModule;
        try {
            guestModule = await WebAssembly.instantiate(guestWasmBytes, guestImports);
        } catch (firstErr) {
            try {
                guestModule = await WebAssembly.instantiate(guestWasmBytes, {});
            } catch (secondErr) {
                throw new Error('[SW] Guest WASM instantiation failed (JSPI): ' + firstErr.message);
            }
        }
        guest = guestModule.instance.exports;

        // Wrap actr_handle as promising (returns Promise instead of i32)
        promisingActrHandle = WebAssembly.promising(guest.actr_handle);
        emitSwLog('info', 'guest_bridge_guest_instantiated_jspi', Object.keys(guest));
    } else {
        // ── Sync-only path: stubs for actr_host_invoke ──
        // Guest WASMs that don't make outbound calls (e.g. echo server) work fine.
        // Guest WASMs requiring outbound calls will fail with UNSUPPORTED_OP.
        const guestImports = {
            env: {
                actr_host_invoke: (_frame_ptr, _frame_len, _reply_ptr, _reply_cap, _reply_len_out) => {
                    console.error('[SW Guest] actr_host_invoke called but JSPI not available');
                    return -7; // UNSUPPORTED_OP
                },
                actr_host_self_id: (_buf_ptr, _buf_cap) => -7,
                actr_host_caller_id: (_buf_ptr, _buf_cap) => -7,
                actr_host_request_id: (_buf_ptr, _buf_cap) => -7,
            },
        };

        let guestModule;
        try {
            guestModule = await WebAssembly.instantiate(guestWasmBytes, guestImports);
        } catch (firstErr) {
            try {
                guestModule = await WebAssembly.instantiate(guestWasmBytes, {});
            } catch (secondErr) {
                throw new Error('[SW] Guest WASM instantiation failed: ' + firstErr.message);
            }
        }
        guest = guestModule.instance.exports;
        emitSwLog('info', 'guest_bridge_guest_instantiated', Object.keys(guest));
    }

    // ── 5. Initialize guest (actr_init) ──
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

    // ── 6. Create dispatch callback ──
    // When JSPI is available, actr_handle may suspend (returns Promise).
    // When JSPI is not available, actr_handle is synchronous.
    const actrHandleFn = promisingActrHandle || guest.actr_handle;
    const isAsync = !!promisingActrHandle;

    async function guestDispatchAsync(abiFrameBytes) {
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

            // Call actr_handle (may return Promise with JSPI, or i32 without)
            const result = isAsync
                ? await actrHandleFn(reqPtr, frameData.length, outBuf, outBuf + 4)
                : actrHandleFn(reqPtr, frameData.length, outBuf, outBuf + 4);
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

    // Sync wrapper for backward compatibility when JSPI is not available
    function guestDispatchSync(abiFrameBytes) {
        try {
            const frameData = new Uint8Array(abiFrameBytes);
            emitSwLog('info', 'guest_dispatch_called', frameData.length);

            const reqPtr = guest.actr_alloc(frameData.length);
            if (reqPtr === 0) throw new Error('[SW Guest] actr_alloc failed for request');

            let mem = new Uint8Array(guest.memory.buffer);
            mem.set(frameData, reqPtr);

            const outBuf = guest.actr_alloc(8);
            if (outBuf === 0) throw new Error('[SW Guest] actr_alloc failed for output buffer');

            const result = guest.actr_handle(reqPtr, frameData.length, outBuf, outBuf + 4);
            emitSwLog('info', 'guest_dispatch_actr_handle_result', result);

            mem = new Uint8Array(guest.memory.buffer);
            const view = new DataView(guest.memory.buffer);

            const respPtr = view.getInt32(outBuf, true);
            const respLen = view.getInt32(outBuf + 4, true);
            emitSwLog('info', 'guest_dispatch_resp_ptr_len', { respPtr, respLen });

            let response = null;
            if (result === 0 && respPtr !== 0 && respLen > 0) {
                response = new Uint8Array(guest.memory.buffer.slice(respPtr, respPtr + respLen));
            }

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

    // ── 7. Register guest workload with runtime ──
    // Use async dispatch when JSPI is available (returns Promise → Rust awaits it).
    // Use sync dispatch when JSPI is not available (returns Uint8Array directly).
    const dispatchFn = isAsync ? guestDispatchAsync : guestDispatchSync;
    wasm_bindgen.register_guest_workload(dispatchFn);
    emitSwLog('info', 'guest_bridge_ready', 'Guest workload registered via JS bridge (JSPI=' + hasJSPI + ')');
}

/**
 * Bring up the Service Worker's WASM runtime and workload.
 *
 * Always runs via `loadWithGuestBridge`: hyper (runtime) and workload
 * (guest) are separate artifacts. `runtime_wasm_url` and `package_url` are
 * both required — the SW never loads an unverified monolithic bundle.
 */
async function ensureWasmReady() {
    if (wasmReady) return;

    if (!RUNTIME_CONFIG) {
        throw new Error('[SW] Cannot load WASM: RUNTIME_CONFIG not yet received');
    }

    const packageUrl = RUNTIME_CONFIG.package_url;
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

        if (!runtimeWasmUrl || !packageUrl) {
            throw new Error(
                '[SW] RUNTIME_CONFIG requires both `runtime_wasm_url` and `package_url`'
            );
        }
        await loadWithGuestBridge(packageUrl, runtimeWasmUrl);

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
