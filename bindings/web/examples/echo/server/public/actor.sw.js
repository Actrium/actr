/* Actor-RTC Service Worker entry for echo-server.
 *
 * 此文件加载用户 WASM (echo_server) 并初始化 SW Runtime
 * 
 * WASM 包含:
 * - actr-runtime-sw (框架代码)
 * - echo-server-wasm (用户 Workload)
 */

/* global wasm_bindgen */

// ── Console interception: forward WASM logs to main page ──
// wasm_logger calls console.info/warn/error via wasm-bindgen glue code.
// We intercept these calls to detect echo RPC events and broadcast
// structured events to all client windows for real-time UI metrics.
(function () {
    const _origInfo = console.info;
    const _origWarn = console.warn;
    const _origError = console.error;
    const _origLog = console.log;

    function extractMessage(args) {
        // wasm_logger output format: console.info("%cLEVEL%c source %c message", css1, css2, css3)
        // args[0] is the format string with %c markers; remaining args alternate
        // between CSS style strings and substitution values.
        // We strip the CSS-only arguments to avoid leaking raw styles into the UI.
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

        // Echo service events
        if (msg.includes('📨') && msg.includes('Echo request')) {
            const m = msg.match(/message='([^']*)'/);
            broadcast({ type: 'echo_event', event: 'request', detail: m ? m[1] : '', ts: Date.now() });
        } else if (msg.includes('📤') && msg.includes('Echo response')) {
            const m = msg.match(/reply='([^']*)'/);
            broadcast({ type: 'echo_event', event: 'response', detail: m ? m[1] : '', ts: Date.now() });
        }

        // Forward all meaningful runtime logs
        if (msg.includes('[SW]') || msg.includes('EchoService') || msg.includes('Echo Server') || msg.includes('Registering')) {
            broadcast({ type: 'sw_log', level: 'info', message: msg, ts: Date.now() });
        }
    };

    console.warn = function (...args) {
        _origWarn.apply(console, args);
        const msg = extractMessage(args);
        if (msg.includes('[SW]') || msg.includes('Echo')) {
            broadcast({ type: 'sw_log', level: 'warn', message: msg, ts: Date.now() });
        }
    };

    console.error = function (...args) {
        _origError.apply(console, args);
        const msg = extractMessage(args);
        // All errors are interesting
        broadcast({ type: 'sw_log', level: 'error', message: msg, ts: Date.now() });
        // Also count as echo error if related to echo
        if (msg.includes('Echo') || msg.includes('handle_request') || msg.includes('service')) {
            broadcast({ type: 'echo_event', event: 'error', detail: msg, ts: Date.now() });
        }
    };

    console.log = function (...args) {
        _origLog.apply(console, args);
        const msg = extractMessage(args);
        if (msg.includes('[EchoService]') || msg.includes('[SW]')) {
            broadcast({ type: 'sw_log', level: 'info', message: msg, ts: Date.now() });
        }
    };
})();

/** @type {import('@actr/web').SwRuntimeConfig | null} */
let RUNTIME_CONFIG = null;

let wasmReady = false;
let wsProbeDone = false;

// Per-client port tracking (clientId → MessagePort)
const clientPorts = new Map();

// Browser Client ID → SW clientId mapping for stale detection
const browserToSwClient = new Map();

/**
 * Clean up stale clients whose browser tabs are no longer active.
 *
 * When a page refreshes, the old browser Client ID disappears from
 * self.clients.matchAll(). We detect these orphaned entries and call
 * unregister_client() so the signaling server can clean up the actor
 * registration and free the WebSocket connection.
 */
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
    // Broadcast to all connected client ports
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

async function ensureWasmReady() {
    if (wasmReady) return;

    let runtimeUrl;
    let wasmUrl;
    try {
        // 加载用户 WASM (包含 SW Runtime + Echo Service)
        runtimeUrl = new URL('echo_server.js', self.location).toString();
        wasmUrl = new URL('echo_server_bg.wasm', self.location).toString();

        emitSwLog('info', 'runtime_url', runtimeUrl);

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

        const runtimeRes = await fetch(runtimeUrl, { cache: 'no-store' });
        emitSwLog('info', 'runtime_fetch', {
            url: runtimeUrl,
            status: runtimeRes.status,
            contentType: runtimeRes.headers.get('content-type'),
        });

        const wasmRes = await fetch(wasmUrl, { cache: 'no-store' });
        emitSwLog('info', 'wasm_fetch', {
            url: wasmUrl,
            status: wasmRes.status,
            contentType: wasmRes.headers.get('content-type'),
        });

        try {
            const runtimeText = await runtimeRes.text();
            const patchedText = runtimeText.replace('let wasm_bindgen =', 'self.wasm_bindgen =');
            (0, eval)(patchedText);
            emitSwLog('info', 'eval_loaded', patchedText.length);
        } catch (error) {
            emitSwLog('error', 'eval_failed', String(error));
            throw error;
        }

        emitSwLog('info', 'wasm_bindgen_call', wasmUrl);
        await wasm_bindgen({ module_or_path: wasmUrl });
        emitSwLog('info', 'wasm_bindgen_ready', null);

        // Global init (logger, panic hook) — once
        wasm_bindgen.init_global();

        // 注册 Echo Service Workload (shared handler, once)
        if (typeof wasm_bindgen.register_echo_service === 'function') {
            wasm_bindgen.register_echo_service();
            emitSwLog('info', 'echo_service_registered', null);
        }

        wasmReady = true;
        emitSwLog('info', 'wasm_ready', null);
    } catch (error) {
        console.error('[SW] WASM init failed:', error);
        emitSwLog('error', 'wasm_init_failed', {
            error: String(error),
            name: error && error.name ? error.name : undefined,
            stack: error && error.stack ? error.stack : undefined,
            runtimeUrl,
            wasmUrl,
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
    // 处理 PING 消息
    if (event.data && event.data.type === 'PING') {
        if (event.source && event.source.postMessage) {
            event.source.postMessage({ type: 'PONG' });
        }
        return;
    }

    // 只处理 DOM_PORT_INIT 消息
    if (!event.data || event.data.type !== 'DOM_PORT_INIT') {
        return;
    }

    // 从 transferable 获取端口
    const port = event.data.port;
    const clientId = event.data.clientId;
    if (!port || !clientId) return;

    // Receive runtime config from main thread (sourced from actr-config.ts)
    if (event.data.runtimeConfig && !RUNTIME_CONFIG) {
        RUNTIME_CONFIG = event.data.runtimeConfig;
    }

    // Track this client's port and browser → SW mapping
    clientPorts.set(clientId, port);
    const browserId = event.source && event.source.id;
    if (browserId) {
        browserToSwClient.set(browserId, clientId);
    }

    // Clean up stale clients from previous page loads (e.g. refresh)
    cleanupStaleClients();

    console.log('[SW] port initialized for client:', clientId, 'total:', clientPorts.size);

    // 发送确认
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

    // 设置端口消息处理器
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

    // 激活端口
    port.start();

    // Register this client with its own independent runtime
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
