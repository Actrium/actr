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
 * instantiate the Component Model guest via its jco-transpiled ES module.
 *
 * Hyper (SW runtime) and workload (guest) are separate artifacts. The
 * runtime is loaded from `runtime_wasm_url`; the workload Component from the
 * signed `.actr` at `package_url`. The runtime's
 * `verify_and_extract_actr_package` is the sole verifier — it hard-fails if
 * no static trust anchor is configured, mirroring native Hyper's
 * mandatory-verify contract.
 *
 * Flow:
 *   1. Load runtime WASM + wasm-bindgen JS glue from `runtime_wasm_url`.
 *   2. Fetch the `.actr` bytes.
 *   3. Call `verify_and_extract_actr_package(bytes, trust_json)` in Rust
 *      to verify Ed25519 + binary hash and extract the guest Component
 *      binary (`application/wasm`, Component Model).
 *   4. Dynamically import the companion jco-transpiled ES module at
 *      `package_url + '.jco/<name>.js'` (or `RUNTIME_CONFIG.jco_module_url`
 *      when set). The ES module is produced at build time by
 *      `bindings/web/scripts/transpile-component.sh`.
 *   5. Call the module's `instantiate(getCoreModule, imports)` with an
 *      import object that binds the WIT `actr:workload/host@0.1.0`
 *      functions to the wasm-bindgen exports from `actr-runtime-sw`
 *      (`host_call_raw_async`, `host_discover_async`, etc.).
 *   6. Register `workload.dispatch` with the runtime via
 *      `register_component_workload`.
 */
async function loadWithComponentBridge(packageUrl, runtimeWasmUrl) {
    emitSwLog('info', 'component_bridge_start', { packageUrl, runtimeWasmUrl });

    // ── 1. Load runtime WASM + JS glue ──
    const jsUrl = runtimeWasmUrl.replace(/_bg\.wasm$/, '.js');
    emitSwLog('info', 'component_bridge_runtime_js', jsUrl);

    const jsResp = await fetch(jsUrl, { cache: 'no-store' });
    if (!jsResp.ok) {
        throw new Error('[SW] Failed to fetch runtime JS glue: ' + jsResp.status);
    }
    const jsText = await jsResp.text();
    const patchedText = jsText.replace('let wasm_bindgen =', 'self.wasm_bindgen =');
    (0, eval)(patchedText);
    emitSwLog('info', 'component_bridge_runtime_js_loaded', jsText.length);

    await wasm_bindgen({ module_or_path: runtimeWasmUrl });
    wasm_bindgen.init_global();
    emitSwLog('info', 'component_bridge_runtime_ready', null);

    // ── 2. Fetch + Rust-side verify + extract guest Component ──
    const resp = await fetch(packageUrl, { cache: 'no-store' });
    if (!resp.ok) {
        throw new Error('[SW] Failed to fetch .actr package: ' + resp.status);
    }
    const buffer = await resp.arrayBuffer();
    emitSwLog('info', 'component_bridge_actr_size', buffer.byteLength);

    const trustJson = JSON.stringify(
        (RUNTIME_CONFIG && Array.isArray(RUNTIME_CONFIG.trust)) ? RUNTIME_CONFIG.trust : []
    );
    let extracted;
    try {
        extracted = wasm_bindgen.verify_and_extract_actr_package(
            new Uint8Array(buffer),
            trustJson,
        );
        emitSwLog('info', 'component_bridge_verify_ok', null);
    } catch (verifyError) {
        emitSwLog('error', 'component_bridge_verify_failed', String(verifyError));
        throw verifyError;
    }

    // `extracted.binary` is the verified Component Model wasm. The SW cannot
    // transpile it at runtime (jco is Node-only); the companion jco output
    // must be delivered alongside the .actr as a build-time artifact.
    emitSwLog('info', 'component_bridge_component_bytes', extracted.binary.byteLength);

    // ── 3. Resolve jco-transpiled ES module URL ──
    // Default convention: the build pipeline places the transpile output at
    // `<packageUrl>.jco/<name>.js` (sibling bundle). Callers can override by
    // providing `RUNTIME_CONFIG.jco_module_url` explicitly.
    const jcoModuleUrl =
        (RUNTIME_CONFIG && RUNTIME_CONFIG.jco_module_url) ||
        (packageUrl.replace(/\.actr$/, '') + '.jco/guest.js');
    emitSwLog('info', 'component_bridge_jco_url', jcoModuleUrl);

    // Service Workers can dynamically import ES modules on recent browsers.
    // If the host environment does not support `import()` inside an SW
    // (older browsers), the page is expected to fall back to a non-SW
    // runtime; we surface a clear error here.
    let jcoModule;
    try {
        jcoModule = await import(/* webpackIgnore: true */ jcoModuleUrl);
    } catch (e) {
        emitSwLog('error', 'component_bridge_jco_import_failed', String(e));
        throw new Error(
            '[SW] Failed to import jco-transpiled component module at ' + jcoModuleUrl +
            ': ' + e.message + '. Ensure scripts/transpile-component.sh has been run ' +
            'against the Component .wasm and its output is served alongside the .actr.'
        );
    }

    // ── 4. Build the host import object bound to runtime-sw wasm-bindgen ──
    //
    // jco's `instantiate()` expects an ImportObject keyed by the fully-
    // qualified WIT interface name (`actr:workload/host@0.1.0`). Each
    // field maps to a function matching the WIT signature; runtime-sw
    // provides the Rust implementations as async wasm-bindgen exports.
    const hostImports = {
        'actr:workload/host@0.1.0': {
            call: async (target, routeKey, payload) =>
                await wasm_bindgen.host_call_async(target, routeKey, payload),
            tell: async (target, routeKey, payload) =>
                await wasm_bindgen.host_tell_async(target, routeKey, payload),
            callRaw: async (target, routeKey, payload) =>
                await wasm_bindgen.host_call_raw_async(target, routeKey, payload),
            discover: async (targetType) =>
                await wasm_bindgen.host_discover_async(targetType),
            logMessage: (level, message) =>
                wasm_bindgen.host_log_message(level, message),
            getSelfId: () => wasm_bindgen.host_get_self_id(),
            getCallerId: () => wasm_bindgen.host_get_caller_id(),
            getRequestId: () => wasm_bindgen.host_get_request_id(),
        },
    };

    // ── 5. Instantiate the Component via jco ──
    //
    // The transpile output's `getCoreModule(path)` callback must return the
    // compiled `WebAssembly.Module` for each core wasm emitted alongside
    // the main module (typically `<name>.core.wasm` plus adapter modules).
    // The sibling files live in the same directory as the JS module.
    const jcoBaseUrl = new URL('.', jcoModuleUrl);
    async function getCoreModule(path) {
        const moduleUrl = new URL(path, jcoBaseUrl);
        const moduleResp = await fetch(moduleUrl, { cache: 'no-store' });
        if (!moduleResp.ok) {
            throw new Error('[SW] Failed to fetch core wasm ' + moduleUrl + ': ' + moduleResp.status);
        }
        const moduleBytes = await moduleResp.arrayBuffer();
        return await WebAssembly.compile(moduleBytes);
    }

    let exports_;
    try {
        const instantiated = jcoModule.instantiate(getCoreModule, hostImports);
        exports_ = instantiated instanceof Promise ? await instantiated : instantiated;
        emitSwLog('info', 'component_bridge_instantiated', Object.keys(exports_));
    } catch (e) {
        emitSwLog('error', 'component_bridge_instantiate_failed', String(e));
        throw new Error('[SW] Component instantiation failed: ' + e.message);
    }

    // ── 6. Register the guest dispatch function ──
    //
    // jco exposes the guest's `workload` interface either by its fully-
    // qualified WIT name or the short alias. Probe both for robustness.
    const workloadExport =
        exports_['actr:workload/workload@0.1.0'] ||
        exports_['workload'];
    if (!workloadExport || typeof workloadExport.dispatch !== 'function') {
        throw new Error(
            '[SW] jco-transpiled component is missing `workload.dispatch` export'
        );
    }

    const dispatchFn = async (envelope) => {
        const result = workloadExport.dispatch(envelope);
        // jco async exports return a Promise; await unconditionally.
        return await result;
    };

    wasm_bindgen.register_component_workload(dispatchFn);
    emitSwLog('info', 'component_bridge_ready', 'Component workload registered');
}

/**
 * Bring up the Service Worker's WASM runtime and workload.
 *
 * Always runs via `loadWithComponentBridge`: hyper (runtime) and workload
 * (Component Model guest + jco-transpiled ES module) are separate artifacts.
 * `runtime_wasm_url` and `package_url` are both required — the SW never
 * loads an unverified monolithic bundle.
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
        await loadWithComponentBridge(packageUrl, runtimeWasmUrl);

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
