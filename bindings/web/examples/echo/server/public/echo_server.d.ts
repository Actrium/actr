declare namespace wasm_bindgen {
    /* tslint:disable */
    /* eslint-disable */

    /**
     * Handle an RPC control request from the DOM side.
     *
     * Unified-dispatcher flow:
     * - With `SERVICE_HANDLER`: `DOM -> handler(route_key, payload, ctx) -> response`
     *   - Local route: the handler processes locally and may call remote targets via `ctx.call_raw()`
     *   - Remote route: the handler forwards to a remote actor via `ctx.call_raw()`
     * - Without `SERVICE_HANDLER`: `DOM -> HostGate -> Gate -> WebRTC` (legacy compatibility path)
     */
    export function handle_dom_control(client_id: string, payload: any): Promise<void>;

    export function handle_dom_fast_path(client_id: string, payload: any): void;

    export function handle_dom_webrtc_event(client_id: string, payload: any): Promise<void>;

    /**
     * WASM initialization entry point
     */
    export function init(): void;

    export function init_global(): void;

    /**
     * Register a new client (browser tab) with the SW runtime.
     *
     * Each call creates an independent runtime with its own signaling connection,
     * actor registration, and WebRTC state. This enables multiple browser tabs
     * to work simultaneously without interfering with each other.
     */
    export function register_client(client_id: string, config: any, port: MessagePort): Promise<void>;

    /**
     * Register a dedicated DataChannel `MessagePort` received from the DOM side.
     *
     * After the DOM creates the DataChannel bridge:
     * 1. DOM: `port1 <-> DataChannel` for bidirectional forwarding
     * 2. DOM: transfers `port2` to the SW via a transferable object
     * 3. SW: this function receives `port2`, builds `WebRtcConnection`, and injects it into `WirePool`
     *
     * After injection, `DestTransport` is awakened through `ReadyWatcher`, and
     * subsequent outbound traffic is sent zero-copy through `DataLane::PostMessage(port)`.
     */
    export function register_datachannel_port(client_id: string, peer_id: string, port: MessagePort): Promise<void>;

    /**
     * Register the `EchoService` handler.
     *
     * The handler dispatches RPC requests to the concrete `EchoService` methods
     * and forwards the `RuntimeContext` into each method.
     */
    export function register_echo_service(): void;

    /**
     * Unregister a client (browser tab) from the SW runtime.
     *
     * Closes the signaling WebSocket (so the signaling server removes
     * the actor from its ServiceRegistry) and removes the client context.
     * Background tasks (signaling relay, heartbeat) will naturally stop
     * when the signaling connection drops.
     */
    export function unregister_client(client_id: string): Promise<void>;

}
declare type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

declare interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly init: () => void;
    readonly register_echo_service: () => void;
    readonly handle_dom_control: (a: number, b: number, c: any) => any;
    readonly handle_dom_fast_path: (a: number, b: number, c: any) => [number, number];
    readonly handle_dom_webrtc_event: (a: number, b: number, c: any) => any;
    readonly init_global: () => [number, number];
    readonly register_client: (a: number, b: number, c: any, d: any) => any;
    readonly register_datachannel_port: (a: number, b: number, c: number, d: number, e: any) => any;
    readonly unregister_client: (a: number, b: number) => any;
    readonly wasm_bindgen__closure__destroy__h5a05f2a88d4b9300: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__hff5f1365318bebd6: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h329e548a2f539608: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h2bf42a8b2ffdb9c9: (a: number, b: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__hf5ee9d21c8541ba9: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen__convert__closures_____invoke__h4fb9192c223e57b8: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_1: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_2: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_3: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_4: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h4b2b0d0c520d2fe0: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h9584e8edcc7fcc5d: (a: number, b: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
declare function wasm_bindgen (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
