/**
 * Echo Server - Actor-RTC Web 浏览器端服务器示例
 *
 * 演示如何使用 @actr/web 统一 Actor API 创建浏览器端服务：
 * 1. 使用 createActor 创建 Actor（初始化 SW Bridge + WebRTC）
 * 2. WASM Service Worker 中的 EchoService Workload 处理实际 RPC
 * 3. SW 通过 console interception + clients.postMessage 将事件广播回主页面
 *
 * 指标数据来源（真实路径）：
 * ┌─────────────────────────────────────────────────────────────┐
 * │  1. WASM 中 echo_service.rs 使用 log::info! 打印日志         │
 * │  2. wasm_logger → console.info() (在 SW 上下文中)            │
 * │  3. actor.sw.js 拦截 console.info, 检测 📨/📤 标记          │
 * │  4. 通过 self.clients.postMessage 广播到主页面               │
 * │  5. 主页面 navigator.serviceWorker.onmessage 接收并更新 UI   │
 * └─────────────────────────────────────────────────────────────┘
 */

import { createActor, type Actor } from '@actr/web';
import { actrConfig, actorType, system } from './generated';

// ── DOM Elements ──
const statusEl = document.getElementById('status')!;
const serviceNameEl = document.getElementById('serviceName')!;
const signalingUrlEl = document.getElementById('signalingUrl')!;
const realmIdEl = document.getElementById('realmId')!;
const actorTypeEl = document.getElementById('actorType')!;
const requestCountEl = document.getElementById('requestCount')!;
const successCountEl = document.getElementById('successCount')!;
const errorCountEl = document.getElementById('errorCount')!;
const logEl = document.getElementById('log')!;

let actor: Actor | null = null;

// ── Real metrics counters (updated by SW events) ──
let requestCount = 0;
let successCount = 0;
let errorCount = 0;

/**
 * 记录日志到 UI
 */
function log(level: 'info' | 'success' | 'warn' | 'error', message: string): void {
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
    logEl.appendChild(entry);
    logEl.scrollTop = logEl.scrollHeight;

    // Keep log size reasonable
    while (logEl.children.length > 200) {
        logEl.removeChild(logEl.firstChild!);
    }
}

function escapeHtml(s: string): string {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/**
 * 更新统计数字显示
 */
function updateStatsUI(): void {
    requestCountEl.textContent = String(requestCount);
    successCountEl.textContent = String(successCount);
    errorCountEl.textContent = String(errorCount);
}

/**
 * 更新服务器信息显示
 */
function updateServerInfo(): void {
    serviceNameEl.textContent = `echo.${actorType.name}`;
    signalingUrlEl.textContent = system.signaling.url;
    realmIdEl.textContent = String(system.deployment.realm_id);
    actorTypeEl.textContent = actorType.fullType;
}

/**
 * 监听来自 Service Worker 的真实事件
 *
 * actor.sw.js 中的 console interception 会通过
 * self.clients.postMessage 广播 echo_event 和 sw_log 消息
 */
function setupSwEventListener(): void {
    navigator.serviceWorker.addEventListener('message', (event) => {
        const data = event.data;
        if (!data || !data.type) return;

        switch (data.type) {
            case 'echo_event':
                handleEchoEvent(data);
                break;
            case 'sw_log':
                handleSwLog(data);
                break;
            // Ignore PONG, sw_ack, etc.
        }
    });
}

/**
 * 处理真实的 Echo RPC 事件（来自 WASM echo_service.rs 的 log::info!）
 */
function handleEchoEvent(data: {
    event: 'request' | 'response' | 'error';
    detail: string;
    ts: number;
}): void {
    switch (data.event) {
        case 'request':
            requestCount++;
            log('info', `📨 收到 Echo 请求: "${data.detail}"`);
            break;
        case 'response':
            successCount++;
            log('success', `✅ 发送 Echo 响应: "${data.detail}"`);
            break;
        case 'error':
            errorCount++;
            log('error', `❌ 处理错误: ${data.detail}`);
            break;
    }
    updateStatsUI();
}

/**
 * 处理 SW 运行时日志（来自 WASM runtime 的 log::info/warn/error）
 */
function handleSwLog(data: {
    level: 'info' | 'warn' | 'error';
    message: string;
    ts: number;
}): void {
    const levelMap: Record<string, 'info' | 'success' | 'warn' | 'error'> = {
        info: 'info',
        warn: 'warn',
        error: 'error',
    };
    const uiLevel = levelMap[data.level] || 'info';

    // Shorten noisy log prefixes for cleaner display
    let msg = data.message;
    msg = msg.replace(/^\s*INFO\s+/, '');
    msg = msg.replace(/^\s*WARN\s+/, '⚠️ ');
    msg = msg.replace(/^\s*ERROR\s+/, '❌ ');

    log(uiLevel, msg);
}

/**
 * 初始化并启动服务器
 */
async function startServer(): Promise<void> {
    try {
        log('info', '🚀 正在启动 Echo Server...');
        log('info', '📦 WASM 将由 Service Worker 加载 (actor.sw.js)');

        // 更新 UI 显示服务器信息
        updateServerInfo();

        // 设置 SW 事件监听（在 createActor 之前，
        // 这样可以捕获初始化过程中的日志）
        setupSwEventListener();

        // 创建 Actor — 这会初始化 SW Bridge 和 WebRTC
        // 实际 RPC 处理由 WASM Service Worker 中的 Rust EchoService Workload 完成
        actor = await createActor({
            ...actrConfig,
            serviceWorkerPath: '/actor.sw.js',
        });

        // 更新状态
        statusEl.innerHTML = '<span>✅</span><span>服务器运行中</span>';
        statusEl.className = 'status ready';

        log('success', '✅ Echo Server 启动成功!');
        log('info', '📡 已注册服务: echo.EchoService');
        log('info', '⏳ 等待客户端连接...');

        console.log('Echo Server started successfully');
    } catch (error) {
        console.error('Failed to start server:', error);
        log('error', `❌ 启动失败: ${(error as Error).message}`);

        statusEl.innerHTML = `<span>❌</span><span>启动失败: ${(error as Error).message}</span>`;
        statusEl.className = 'status error';
    }
}

// 页面卸载时关闭 Actor
window.addEventListener('beforeunload', async () => {
    if (actor) {
        await actor.close();
    }
});

// Keep Service Worker alive by sending periodic pings.
// SW will be terminated by the browser if idle for ~30s.
let keepAliveTimer: ReturnType<typeof setInterval> | null = null;
function startKeepAlive(): void {
    if (keepAliveTimer) return;
    keepAliveTimer = setInterval(() => {
        navigator.serviceWorker.controller?.postMessage({ type: 'PING' });
    }, 20_000); // every 20s
}

// 启动服务器
startServer().then(() => {
    startKeepAlive();
});
