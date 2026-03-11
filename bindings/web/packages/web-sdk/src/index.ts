/**
 * Actor-RTC Web SDK
 *
 * High-level TypeScript SDK for Actor-RTC Web platform
 *
 *  WASM-DOM ：
 * - DOM ：@actr/dom ()
 * - Service Worker ：WASM 
 * - UI ： SDK  WASM 
 */

export * from './actor';
export * from './config';
export * from './errors';
export * from './types';
export * from './actor-ref';

// Unified API
export { createActor } from './actor';
export type { Actor, ActorConfig } from './actor';
