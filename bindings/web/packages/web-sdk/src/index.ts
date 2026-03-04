/**
 * Actor-RTC Web SDK
 *
 * High-level TypeScript SDK for Actor-RTC Web platform
 *
 * 基于 WASM-DOM 集成架构：
 * - DOM 侧：@actr/dom (固定转发层)
 * - Service Worker 侧：WASM 运行时
 * - UI 侧：通过此 SDK 与 WASM 交互
 */

export * from './actor';
export * from './config';
export * from './errors';
export * from './types';
export * from './actor-ref';

// Unified API
export { createActor } from './actor';
export type { Actor, ActorConfig } from './actor';
