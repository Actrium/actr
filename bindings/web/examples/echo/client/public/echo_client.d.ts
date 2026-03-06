declare namespace wasm_bindgen {
    /* tslint:disable */
    /* eslint-disable */

    /**
     * 处理来自 DOM 的 RPC 控制请求
     *
     * 消息流（Unified Dispatcher 模式）：
     * - 有 SERVICE_HANDLER: DOM → handler(route_key, payload, ctx) → response
     *   - local route: handler 本地处理，可通过 ctx.call_raw() 调远程
     *   - remote route: handler 通过 ctx.call_raw() 转发到远程 Actor
     * - 无 SERVICE_HANDLER: DOM → InprocOutGate → OutGate → WebRTC（旧路径，向后兼容）
     */
    export function handle_dom_control(client_id: string, payload: any): Promise<void>;

    export function handle_dom_fast_path(client_id: string, payload: any): void;

    export function handle_dom_webrtc_event(client_id: string, payload: any): Promise<void>;

    /**
     * WASM 初始化入口
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
     * 注册来自 DOM 的专用 DataChannel MessagePort
     *
     * DOM 在 DataChannel 建立后创建 MessageChannel 桥接：
     * 1. DOM: `port1 ↔ DataChannel` (双向转发)
     * 2. DOM: 将 `port2` 通过 Transferable 转移给 SW
     * 3. SW: 此函数接收 `port2`，创建 `WebRtcConnection`，注入到 `WirePool`
     *
     * 注入后 DestTransport 的 send 循环通过 ReadyWatcher 被唤醒，
     * 后续出站数据直接经 `DataLane::PostMessage(port)` 零拷贝发送。
     */
    export function register_datachannel_port(client_id: string, peer_id: string, port: MessagePort): Promise<void>;

    /**
     * 注册本地 SendEcho 服务 handler
     *
     * handler 分发 RPC 请求到 SendEcho 服务的具体方法：
     * - `echo.SendEcho.SendEcho`: 发现远程 EchoService 并转发请求
     */
    export function register_echo_client_handler(): void;

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
    readonly register_echo_client_handler: () => void;
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
