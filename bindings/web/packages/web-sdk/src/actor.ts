/**
 * Unified Actor API
 *
 * P2P ， client/server。 Actor ：
 * -  Signaling、 WebRTC 
 * -  WASM handler（UnifiedDispatcher）
 * -  callRaw/call  Actor
 *
 *  Kotlin  UnifiedHandler + ContextBridge 。
 */

import { initActrDom, type ActrDomRuntime } from '@actr/dom';
import { ActorRef } from './actor-ref';
import type { ActorClientConfig } from './config';
import { DEFAULT_CONFIG } from './config';
import { ActorError } from './errors';
import type {
    ConnectionState,
    RpcOptions,
    SubscriptionCallback,
    UnsubscribeFn,
} from './types';

/**
 * Actor 
 */
export interface ActorConfig extends ActorClientConfig {
    /**
     * WASM （）
     *
     * ， Service Worker  WASM  handler。
     * WASM  handler  register_service_handler() ，
     * （ ctx.call_raw()）。
     */
    wasmUrl?: string;
}

/**
 * Actor 
 *
 *  P2P Actor，。
 */
export class Actor {
    private config: Required<ActorConfig>;
    private domRuntime: ActrDomRuntime | null = null;
    private actorRef: ActorRef | null = null;
    private connectionState: ConnectionState;
    private eventListeners: Map<string, Set<(...args: unknown[]) => void>>;

    private constructor(config: Required<ActorConfig>) {
        this.config = config;
        this.connectionState = 'disconnected' as ConnectionState;
        this.eventListeners = new Map();
    }

    /**
     *  Actor 
     */
    static async create(config: ActorConfig): Promise<Actor> {
        const fullConfig = {
            ...DEFAULT_CONFIG,
            ...config,
            retry: {
                ...DEFAULT_CONFIG.retry,
                ...config.retry,
            },
        } as Required<ActorConfig>;

        if (!fullConfig.signalingUrl) {
            throw ActorError.configError('signalingUrl is required');
        }
        if (!fullConfig.realm) {
            throw ActorError.configError('realm is required');
        }

        const actor = new Actor(fullConfig);
        await actor.initialize();
        return actor;
    }

    /**
     * 
     */
    private async initialize(): Promise<void> {
        try {
            this.connectionState = 'connecting' as ConnectionState;
            this.emit('stateChange', this.connectionState);

            this.domRuntime = await initActrDom({
                serviceWorkerUrl: this.config.serviceWorkerPath || '/actor.sw.js',
                webrtcConfig: {
                    iceServers: this.config.iceServers || [{ urls: 'stun:stun.l.google.com:19302' }],
                    iceTransportPolicy: this.config.iceTransportPolicy,
                },
                runtimeConfig: this.config.runtimeConfig as unknown as Record<string, unknown> | undefined,
            });

            this.actorRef = new ActorRef(this.domRuntime);

            this.actorRef.on('stateChange', ((state: ConnectionState) => {
                this.connectionState = state;
                this.emit('stateChange', state);
            }) as (...args: unknown[]) => void);

            this.connectionState = 'connected' as ConnectionState;
            this.emit('stateChange', this.connectionState);

            if (this.config.debug) {
                console.log('[Actor] Initialized successfully');
            }
        } catch (error) {
            this.connectionState = 'failed' as ConnectionState;
            this.emit('stateChange', this.connectionState);
            throw ActorError.fromWasmError(error);
        }
    }

    /**
     *  ActorRef
     */
    getActorRef(): ActorRef {
        if (!this.actorRef) {
            throw ActorError.connectionError('Actor not initialized');
        }
        return this.actorRef;
    }

    /**
     *  RPC（ payload）
     */
    async callRaw(routeKey: string, payload: Uint8Array, timeout?: number): Promise<Uint8Array> {
        if (!this.actorRef) {
            throw ActorError.connectionError('Actor not initialized');
        }

        try {
            const response = await this.actorRef.callRaw(routeKey, payload, timeout);
            if (this.config.debug) {
                console.log(`[Actor] callRaw ${routeKey}`);
            }
            return response;
        } catch (error) {
            throw ActorError.fromWasmError(error);
        }
    }

    /**
     *  RPC 
     */
    async call<TRequest, TResponse>(
        service: string,
        method: string,
        request: TRequest,
        options?: RpcOptions
    ): Promise<TResponse> {
        if (!this.actorRef) {
            throw ActorError.connectionError('Actor not initialized');
        }

        try {
            const timeout = options?.timeout || this.config.connectionTimeout;
            const response = await this.actorRef.call<TRequest, TResponse>(
                service,
                method,
                request,
                timeout
            );

            if (this.config.debug) {
                console.log(`[Actor] Called ${service}.${method}`);
            }
            return response;
        } catch (error) {
            throw ActorError.fromWasmError(error);
        }
    }

    /**
     * 
     */
    async subscribe<T>(topic: string, callback: SubscriptionCallback<T>): Promise<UnsubscribeFn> {
        if (!this.actorRef) {
            throw ActorError.connectionError('Actor not initialized');
        }

        try {
            return await this.actorRef.subscribe<T>(topic, callback);
        } catch (error) {
            throw ActorError.fromWasmError(error);
        }
    }

    getConnectionState(): ConnectionState {
        return this.connectionState;
    }

    isConnected(): boolean {
        return this.connectionState === ('connected' as ConnectionState);
    }

    on(event: string, listener: (...args: unknown[]) => void): void {
        if (!this.eventListeners.has(event)) {
            this.eventListeners.set(event, new Set());
        }
        this.eventListeners.get(event)?.add(listener);
    }

    off(event: string, listener: (...args: unknown[]) => void): void {
        this.eventListeners.get(event)?.delete(listener);
    }

    private emit(event: string, ...args: unknown[]): void {
        this.eventListeners.get(event)?.forEach((listener) => listener(...args));
    }

    async close(): Promise<void> {
        if (this.actorRef) {
            this.actorRef.dispose();
            this.actorRef = null;
        }
        if (this.domRuntime) {
            this.domRuntime.dispose();
            this.domRuntime = null;
        }
        this.connectionState = 'disconnected' as ConnectionState;
        this.emit('stateChange', this.connectionState);

        if (this.config.debug) {
            console.log('[Actor] Closed');
        }
    }
}

/**
 *  Actor （ API）
 *
 * @example
 * ```typescript
 * import { createActor } from '@actr/web';
 *
 * const actor = await createActor({
 *   signalingUrl: 'ws://localhost:8081/signaling/ws',
 *   realm: 'demo',
 *   serviceWorkerPath: '/actor.sw.js',
 * });
 *
 * //  Actor
 * const response = await actor.callRaw('echo.EchoService.Echo', encoded);
 * ```
 */
export async function createActor(config: ActorConfig): Promise<Actor> {
    return Actor.create(config);
}
