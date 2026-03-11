/**
 * Actor-RTC Web Hello World Example
 *
 * Demonstrates how to use the @actr/web SDK to call a remote Echo service.
 *
 * Features:
 * 1. Connect to the signaling server
 * 2. Invoke Echo service via a WebRTC DataChannel
 * 3. Display request/response results
 */

import { createActor, Actor } from '@actr/web';
import { actrConfig, EchoServiceActorRef } from './generated';
import { EchoRequest, EchoResponse } from './generated/remote/echo-echo-server/echo';

// DOM elements
const statusEl = document.getElementById('status')!;
const sendBtn = document.getElementById('sendBtn') as HTMLButtonElement;
const resultEl = document.getElementById('result')!;

let actor: Actor | null = null;
let echoService: EchoServiceActorRef | null = null;

/**
 * Display logs on the page
 */
function log(message: string): void {
  console.log(`[HelloWorld] ${message}`);
  resultEl.innerHTML += `<div>${new Date().toLocaleTimeString()}: ${message}</div>`;
}

/**
 * Initialize the client
 */
async function init() {
  try {
    statusEl.textContent = 'Connecting...';
    statusEl.className = 'status connecting';

    // Create an actor using the shared Actor API
    actor = await createActor({
      ...actrConfig,
      serviceWorkerPath: '/actor.sw.js?v=7',
      debug: true,
    });

    // Create a type-safe Echo service reference
    echoService = new EchoServiceActorRef(actor);

    statusEl.textContent = '✅ Connected';
    statusEl.className = 'status connected';
    sendBtn.disabled = false;

    // Monitor connection state
    actor.on('stateChange', (state) => {
      console.log('Connection state:', state);
      statusEl.textContent = `Connection status: ${state}`;
    });

    console.log('Client initialized successfully');

    // Auto-test: send an echo message after 5 seconds
    log('⏳ Will automatically send Echo test message in 5s...');
    setTimeout(async () => {
      log('🚀 Automatically sending Echo test message...');
      await sendEcho();
    }, 5000);
  } catch (error) {
    console.error('Failed to initialize client:', error);
    statusEl.textContent = `❌ Connection failed: ${(error as Error).message}`;
    statusEl.className = 'status error';
  }
}

/**
 * Send an Echo message
 *
 * Demonstrates two invocation paths:
 * 1. Type-safe method from `ActorRef`
 * 2. Lower-level `callRaw` (when the ActorRef is unavailable)
 */
async function sendEcho() {
  if (!actor || !echoService) {
    resultEl.textContent = 'Client is not initialized';
    return;
  }

  try {
    sendBtn.disabled = true;
    resultEl.textContent = 'Sending...';

    const message = `Hello from Actor-RTC Web! (${new Date().toLocaleTimeString()})`;

    // Option 1: use the type-safe ActorRef (recommended)
    const response = await echoService.echo({ message });

    // Option 2: the low-level callRaw (when the ActorRef is unavailable)
    // const request: EchoRequest = { message };
    // const encoded = EchoRequest.encode(request).finish();
    // const responseData = await client.callRaw("echo.EchoService.Echo", encoded);
    // const response: EchoResponse = EchoResponse.decode(responseData);

    resultEl.innerHTML = `
      <strong>Sent:</strong> ${message}<br>
      <strong>Reply:</strong> ${response.reply}<br>
      <strong>Timestamp:</strong> ${new Date(Number(response.timestamp) * 1000).toLocaleString()}
    `;

    console.log('Echo response:', response);
  } catch (error) {
    console.error('Failed to send echo:', error);
    resultEl.textContent = `Error: ${(error as Error).message}`;
  } finally {
    sendBtn.disabled = false;
  }
}

// Event listeners
sendBtn.addEventListener('click', sendEcho);

// Cleanup on page unload
window.addEventListener('beforeunload', async () => {
  if (actor) {
    await actor.close();
  }
});

// Start the application
init();
