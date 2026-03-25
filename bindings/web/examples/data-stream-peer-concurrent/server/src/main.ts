import { createActor, type Actor } from '@actr/web';
import { actrConfig } from './config';

const statusEl = document.getElementById('status')!;
const logEl = document.getElementById('log')!;

let actor: Actor | null = null;

function appendLog(message: string): void {
    const ts = new Date().toLocaleTimeString('en-US', { hour12: false });
    logEl.textContent += `[${ts}] ${message}\n`;
    logEl.scrollTop = logEl.scrollHeight;
}

function setStatus(text: string, ok = true): void {
    statusEl.textContent = text;
    statusEl.className = ok ? 'ok' : 'err';
}

function setupSwLogListener(): void {
    navigator.serviceWorker.addEventListener('message', (event) => {
        const data = event.data;
        if (!data || data.type !== 'sw_log') return;
        appendLog(String(data.message || ''));
    });
}

async function init(): Promise<void> {
    setupSwLogListener();
    setStatus('Connecting...', true);
    appendLog('initializing server actor...');

    try {
        actor = await createActor(actrConfig);
        setStatus('✅ Server running', true);
        appendLog('server actor initialized');
        actor.on('stateChange', (state) => appendLog(`stateChange: ${String(state)}`));
    } catch (error) {
        setStatus(`❌ Startup failed: ${(error as Error).message}`, false);
        appendLog(`init failed: ${(error as Error).message}`);
    }
}

window.addEventListener('beforeunload', async () => {
    if (actor) await actor.close();
});

setInterval(() => {
    navigator.serviceWorker.controller?.postMessage({ type: 'PING' });
}, 20_000);

void init();
