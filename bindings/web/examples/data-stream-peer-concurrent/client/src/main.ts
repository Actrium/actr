import { createActor, type Actor } from '@actr/web';
import { actrConfig } from './config';
import { StreamClientActorRef } from './actorref';

const statusEl = document.getElementById('status')!;
const logEl = document.getElementById('log')!;
const startBtn = document.getElementById('startBtn') as HTMLButtonElement;
const clientIdInput = document.getElementById('clientId') as HTMLInputElement;
const messageCountInput = document.getElementById('messageCount') as HTMLInputElement;

let actor: Actor | null = null;
let streamClient: StreamClientActorRef | null = null;
let started = false;

function appendLog(message: string): void {
    const ts = new Date().toLocaleTimeString('zh-CN', { hour12: false });
    logEl.textContent += `[${ts}] ${message}\n`;
    logEl.scrollTop = logEl.scrollHeight;
}

function setStatus(text: string, ok = true): void {
    statusEl.textContent = text;
    statusEl.className = ok ? 'ok' : 'err';
}

function readQuery(): void {
    const params = new URLSearchParams(window.location.search);
    const clientId = params.get('clientId');
    const messageCount = params.get('messageCount');
    if (clientId) clientIdInput.value = clientId;
    if (messageCount) messageCountInput.value = messageCount;
}

function setupSwLogListener(): void {
    navigator.serviceWorker.addEventListener('message', (event) => {
        const data = event.data;
        if (!data || data.type !== 'sw_log') return;
        appendLog(String(data.message || ''));
    });
}

async function startStream(): Promise<void> {
    if (!streamClient || started) return;
    started = true;
    startBtn.disabled = true;

    const clientId = clientIdInput.value.trim() || 'client-1';
    const messageCount = Math.max(1, Number(messageCountInput.value || 3));
    const stream_id = `${clientId}-stream`;

    appendLog(`start_stream request: client_id=${clientId} stream_id=${stream_id} message_count=${messageCount}`);

    try {
        const response = await streamClient.startStream({
            client_id: clientId,
            stream_id,
            message_count: messageCount,
        });
        appendLog(`start_stream response: accepted=${response.accepted} message=${response.message}`);
    } catch (error) {
        appendLog(`start_stream failed: ${(error as Error).message}`);
        setStatus(`启动失败: ${(error as Error).message}`, false);
    } finally {
        started = false;
        startBtn.disabled = false;
    }
}

async function init(): Promise<void> {
    readQuery();
    setupSwLogListener();
    setStatus('连接中...', true);
    appendLog('initializing actor...');

    try {
        actor = await createActor(actrConfig);
        streamClient = new StreamClientActorRef(actor);
        setStatus('✅ 已连接', true);
        appendLog('actor initialized');
        startBtn.disabled = false;

        actor.on('stateChange', (state) => appendLog(`stateChange: ${String(state)}`));

        const params = new URLSearchParams(window.location.search);
        if (params.get('autoStart') === '1') {
            setTimeout(() => {
                void startStream();
            }, 4000);
        }
    } catch (error) {
        setStatus(`❌ 连接失败: ${(error as Error).message}`, false);
        appendLog(`init failed: ${(error as Error).message}`);
    }
}

startBtn.addEventListener('click', () => {
    void startStream();
});

window.addEventListener('beforeunload', async () => {
    if (actor) await actor.close();
});

setInterval(() => {
    navigator.serviceWorker.controller?.postMessage({ type: 'PING' });
}, 20_000);

void init();
