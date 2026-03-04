/**
 * Echo Client - Actor-RTC Web 浏览器客户端示例
 *
 * 演示如何使用 @actr/web 统一 Actor API + Local Handler 调用远程 Echo 服务：
 * 1. 创建 Actor（统一 P2P 实例，自动加载含 Local Handler 的 WASM）
 * 2. DOM 通过 callRaw 发送请求
 * 3. Local Handler (WASM) 通过 ctx.discover() 发现远端 Echo Server
 * 4. Local Handler 通过 ctx.call_raw() 转发请求到远端并返回响应
 */

import { createActor, Actor } from '@actr/web';
import { actrConfig, EchoServiceActorRef } from './generated';

// ── DOM Elements ──
const statusEl = document.getElementById('status')!;
const sendBtn = document.getElementById('sendBtn') as HTMLButtonElement;
const msgInput = document.getElementById('msgInput') as HTMLInputElement;
const resultEl = document.getElementById('result')!;
const swLogEl = document.getElementById('swLog')!;

let actor: Actor | null = null;
let echoService: EchoServiceActorRef | null = null;

/**
 * 在 Echo 结果区显示日志
 */
function log(level: 'info' | 'ok' | 'err', message: string): void {
    const time = new Date().toLocaleTimeString('zh-CN', {
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
 * 在 SW Runtime 日志面板显示日志
 */
function swLog(level: 'info' | 'success' | 'warn' | 'error', message: string): void {
    const time = new Date().toLocaleTimeString('zh-CN', {
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
 * 初始化客户端
 */
async function init(): Promise<void> {
    try {
        statusEl.textContent = '连接中...';
        statusEl.className = 'status connecting';

        log('info', '🚀 正在初始化 Echo Client...');

        // 使用统一 Actor API 创建实例
        // Service Worker 中的 WASM 已包含 Local Handler，
        // 它会自动处理 discover + call_raw 转发
        actor = await createActor({
            ...actrConfig,
            serviceWorkerPath: '/actor.sw.js',
            debug: true,
        });

        // 创建类型安全的 Echo 服务引用
        echoService = new EchoServiceActorRef(actor);

        statusEl.textContent = '✅ 已连接';
        statusEl.className = 'status connected';
        sendBtn.disabled = false;

        // 监听连接状态
        actor.on('stateChange', (state) => {
            log('info', `连接状态: ${state}`);
        });

        log('ok', '✅ 客户端初始化成功');
        log('info', '⏳ 将在 5 秒后自动发送 Echo 测试消息...');

        // 自动测试
        setTimeout(async () => {
            log('info', '🚀 自动发送 Echo 测试消息...');
            await sendEcho();
        }, 5000);
    } catch (error) {
        console.error('Failed to initialize client:', error);
        statusEl.textContent = `❌ 连接失败: ${(error as Error).message}`;
        statusEl.className = 'status error';
        log('err', `❌ 初始化失败: ${(error as Error).message}`);
    }
}

/**
 * 发送 Echo 消息
 */
async function sendEcho(): Promise<void> {
    if (!actor || !echoService) {
        log('err', '❌ 客户端未初始化');
        return;
    }

    const message = msgInput.value.trim() || `Hello! (${new Date().toLocaleTimeString()})`;

    try {
        sendBtn.disabled = true;
        log('info', `📤 发送: "${message}"`);

        const response = await echoService.echo({ message });

        log('ok', `📥 回复: "${response.reply}"`);
        log('info', `⏱️ 时间戳: ${new Date(Number(response.timestamp) * 1000).toLocaleString()}`);
    } catch (error) {
        console.error('Echo failed:', error);
        log('err', `❌ 请求失败: ${(error as Error).message}`);
    } finally {
        sendBtn.disabled = false;
    }
}

// ── Event Listeners ──
sendBtn.addEventListener('click', sendEcho);
msgInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !sendBtn.disabled) sendEcho();
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
 * 监听来自 Service Worker 的运行时日志
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

// 尽早设置 SW 日志监听（在 init 之前，以捕获初始化日志）
setupSwLogListener();

// 启动
init();
