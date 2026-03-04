/* Actor-RTC Service Worker entry for hello-world. */

/* global wasm_bindgen */

const RUNTIME_CONFIG = {
  signaling_url: 'wss://10.30.3.206:8081/signaling/ws',
  realm_id: 2368266035,
  client_actr_type: 'acme+echo-client-app',
  target_actr_type: 'acme+EchoService',
  service_fingerprint: '',
  // ACL: allow EchoService to send responses back to this client
  acl_allow_types: ['acme+EchoService'],
  // Client mode (default): will discover server target
  is_server: false,
};

let wasmReady = false;
let wsProbeDone = false;

// Per-client port tracking (clientId → MessagePort)
const clientPorts = new Map();

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

async function ensureWasmReady() {
  if (wasmReady) return;

  let runtimeUrl;
  let wasmUrl;
  try {
    runtimeUrl = new URL('actr_runtime_sw.js', self.location).toString();
    wasmUrl = new URL('actr_runtime_sw_bg.wasm', self.location).toString();
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
    await wasm_bindgen(wasmUrl);
    emitSwLog('info', 'wasm_bindgen_ready', null);
    wasm_bindgen.init_global();
    wasmReady = true;
    emitSwLog('info', 'wasm_ready', null);
  } catch (error) {
    console.error('[SW] runtime init failed:', error);
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
  event.waitUntil(self.skipWaiting());
});

self.addEventListener('activate', (event) => {
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
  clientPorts.set(clientId, port);
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
        break;
    }
  };

  port.start();
  ensureWasmReady().then(async () => {
    try {
      await wasm_bindgen.register_client(clientId, RUNTIME_CONFIG, port);
      emitSwLog('info', 'client_registered', { clientId });
    } catch (error) {
      console.error('[SW] register_client failed:', error);
      emitSwLog('error', 'register_client_failed', { clientId, error: String(error) });
    }
  });
});