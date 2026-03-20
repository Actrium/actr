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
export * from './package-loader';

// Unified API
export { createActor } from './actor';
export type { Actor, ActorConfig } from './actor';
export { loadActrPackage, parseActrPackage } from './package-loader';
export type { ActrManifest, LoadedActrPackage } from './package-loader';
