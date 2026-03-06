/* global wasm_bindgen */

(function () {
    const original = {
        info: console.info.bind(console),
        warn: console.warn.bind(console),
        error: console.error.bind(console),
        log: console.log.bind(console),
    };

    function extractMessage(args) {
        return Array.from(args)
            .filter((arg) => typeof arg === 'string')
            .join(' ')
            .replace(/%c/g, '')
            .trim();
    }

    function broadcast(type, level, message) {
        self.clients.matchAll({ type: 'window' }).then((clients) => {
            for (const client of clients) {
                client.postMessage({ type, level, message, ts: Date.now() });
            }
        }).catch(() => { });
    }

    console.info = (...args) => {
        original.info(...args);
        const msg = extractMessage(args);
        if (msg) broadcast('sw_log', 'info', msg);
    };
    console.warn = (...args) => {
        original.warn(...args);
        const msg = extractMessage(args);
        if (msg) broadcast('sw_log', 'warn', msg);
    };
    console.error = (...args) => {
        original.error(...args);
        const msg = extractMessage(args);
        if (msg) broadcast('sw_log', 'error', msg);
    };
    console.log = (...args) => {
        original.log(...args);
        const msg = extractMessage(args);
        if (msg && (msg.includes('[SW]') || msg.includes('[DataStream'))) {
            broadcast('sw_log', 'info', msg);
        }
    };
})();

let RUNTIME_CONFIG = null;
let wasmReady = false;
const clientPorts = new Map();
const browserToSwClient = new Map();

async function cleanupStaleClients() {
    if (!wasmReady) return;
    try {
        const activeWindows = await self.clients.matchAll({ type: 'window' });
        const activeIds = new Set(activeWindows.map((c) => c.id));
        for (const [browserId, swClientId] of browserToSwClient.entries()) {
            if (!activeIds.has(browserId)) {
                browserToSwClient.delete(browserId);
                clientPorts.delete(swClientId);
                try {
                    await wasm_bindgen.unregister_client(swClientId);
                } catch (_) { }
            }
        }
    } catch (_) { }
}

async function ensureWasmReady() {
    if (wasmReady) return;

    const runtimeUrl = new URL('data_stream_client.js', self.location).toString();
    const wasmUrl = new URL('data_stream_client_bg.wasm', self.location).toString();

    const runtimeRes = await fetch(runtimeUrl, { cache: 'no-store' });
    const runtimeText = await runtimeRes.text();
    (0, eval)(runtimeText.replace('let wasm_bindgen =', 'self.wasm_bindgen ='));
    await wasm_bindgen({ module_or_path: wasmUrl });
    wasm_bindgen.init_global();
    wasm_bindgen.register_stream_client_handler();
    wasmReady = true;
}

self.addEventListener('install', (event) => {
    event.waitUntil(self.skipWaiting());
});

self.addEventListener('activate', (event) => {
    event.waitUntil(self.clients.claim());
});

self.addEventListener('message', (event) => {
    if (event.data?.type === 'DOM_PORT_INIT') {
        const port = event.data.port;
        const clientId = event.data.clientId;
        RUNTIME_CONFIG = event.data.runtimeConfig;

        clientPorts.set(clientId, port);
        const browserId = event.source && event.source.id;
        if (browserId) browserToSwClient.set(browserId, clientId);
        cleanupStaleClients();

        if (event.source && event.source.postMessage) {
            event.source.postMessage({ type: 'sw_ack', message: 'port_ready' });
        }

        port.onmessage = async (portEvent) => {
            await ensureWasmReady();
            const message = portEvent.data;
            if (!message || !message.type) return;

            switch (message.type) {
                case 'control':
                    await wasm_bindgen.handle_dom_control(clientId, message.payload);
                    break;
                case 'webrtc_event':
                    await wasm_bindgen.handle_dom_webrtc_event(clientId, message.payload);
                    break;
                case 'fast_path_data':
                    await wasm_bindgen.handle_dom_fast_path(clientId, message.payload);
                    break;
                case 'register_datachannel_port':
                    if (message.payload?.port && message.payload?.peerId) {
                        await wasm_bindgen.register_datachannel_port(clientId, message.payload.peerId, message.payload.port);
                    }
                    break;
                default:
                    break;
            }
        };

        port.start();
        ensureWasmReady().then(async () => {
            await wasm_bindgen.register_client(clientId, RUNTIME_CONFIG, port);
        }).catch((error) => {
            console.error('[SW] register_client failed', error);
        });
    }
});
