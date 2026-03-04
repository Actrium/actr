/**
 * Actor-RTC Web Hello World Example
 *
 * 演示如何使用 @actr/web SDK 调用远程 Echo 服务
 *
 * 功能:
 * 1. 连接到 signaling server
 * 2. 通过 WebRTC DataChannel 调用 Echo 服务
 * 3. 显示请求/响应结果
 */

import { createActor, Actor } from '@actr/web';
import { actrConfig, EchoServiceActorRef } from './generated';
import { EchoRequest, EchoResponse } from './generated/remote/echo-echo-server/echo';

// DOM 元素
const statusEl = document.getElementById('status')!;
const sendBtn = document.getElementById('sendBtn') as HTMLButtonElement;
const resultEl = document.getElementById('result')!;

let actor: Actor | null = null;
let echoService: EchoServiceActorRef | null = null;

/**
 * 在页面上显示日志
 */
function log(message: string): void {
  console.log(`[HelloWorld] ${message}`);
  resultEl.innerHTML += `<div>${new Date().toLocaleTimeString()}: ${message}</div>`;
}

/**
 * 初始化客户端
 */
async function init() {
  try {
    statusEl.textContent = '连接中...';
    statusEl.className = 'status connecting';

    // 使用统一 Actor API 创建实例
    actor = await createActor({
      ...actrConfig,
      serviceWorkerPath: '/actor.sw.js?v=7',
      debug: true,
    });

    // 创建类型安全的 Echo 服务引用
    echoService = new EchoServiceActorRef(actor);

    statusEl.textContent = '✅ 已连接';
    statusEl.className = 'status connected';
    sendBtn.disabled = false;

    // 监听连接状态
    actor.on('stateChange', (state) => {
      console.log('Connection state:', state);
      statusEl.textContent = `连接状态: ${state}`;
    });

    console.log('Client initialized successfully');

    // 自动测试: 5 秒后自动发送 echo 消息
    log('⏳ 将在 5 秒后自动发送 Echo 测试消息...');
    setTimeout(async () => {
      log('🚀 自动发送 Echo 测试消息...');
      await sendEcho();
    }, 5000);
  } catch (error) {
    console.error('Failed to initialize client:', error);
    statusEl.textContent = `❌ 连接失败: ${(error as Error).message}`;
    statusEl.className = 'status error';
  }
}

/**
 * 发送 Echo 消息
 *
 * 演示两种调用方式:
 * 1. 使用 ActorRef 的类型安全方法
 * 2. 使用 callRaw 的底层方法
 */
async function sendEcho() {
  if (!actor || !echoService) {
    resultEl.textContent = '客户端未初始化';
    return;
  }

  try {
    sendBtn.disabled = true;
    resultEl.textContent = '发送中...';

    const message = `Hello from Actor-RTC Web! (${new Date().toLocaleTimeString()})`;

    // 方式 1: 使用类型安全的 ActorRef (推荐)
    const response = await echoService.echo({ message });

    // 方式 2: 使用底层 callRaw (如果 ActorRef 不可用时)
    // const request: EchoRequest = { message };
    // const encoded = EchoRequest.encode(request).finish();
    // const responseData = await client.callRaw("echo.EchoService.Echo", encoded);
    // const response: EchoResponse = EchoResponse.decode(responseData);

    resultEl.innerHTML = `
      <strong>发送:</strong> ${message}<br>
      <strong>回复:</strong> ${response.reply}<br>
      <strong>时间戳:</strong> ${new Date(Number(response.timestamp) * 1000).toLocaleString()}
    `;

    console.log('Echo response:', response);
  } catch (error) {
    console.error('Failed to send echo:', error);
    resultEl.textContent = `错误: ${(error as Error).message}`;
  } finally {
    sendBtn.disabled = false;
  }
}

// 事件监听
sendBtn.addEventListener('click', sendEcho);

// 页面卸载时清理
window.addEventListener('beforeunload', async () => {
  if (actor) {
    await actor.close();
  }
});

// 启动应用
init();

