let wasm_bindgen = (function(exports) {
    let script_src;
    if (typeof document !== 'undefined' && document.currentScript !== null) {
        script_src = new URL(document.currentScript.src, location.href).toString();
    }

    /**
     * Handle an RPC control request from the DOM side.
     *
     * Unified-dispatcher flow:
     * - With `SERVICE_HANDLER`: `DOM -> handler(route_key, payload, ctx) -> response`
     *   - Local route: the handler processes locally and may call remote targets via `ctx.call_raw()`
     *   - Remote route: the handler forwards to a remote actor via `ctx.call_raw()`
     * - Without `SERVICE_HANDLER`: `DOM -> HostGate -> Gate -> WebRTC` (legacy compatibility path)
     * @param {string} client_id
     * @param {any} payload
     * @returns {Promise<void>}
     */
    function handle_dom_control(client_id, payload) {
        const ptr0 = passStringToWasm0(client_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.handle_dom_control(ptr0, len0, payload);
        return ret;
    }
    exports.handle_dom_control = handle_dom_control;

    /**
     * @param {string} client_id
     * @param {any} payload
     */
    function handle_dom_fast_path(client_id, payload) {
        const ptr0 = passStringToWasm0(client_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.handle_dom_fast_path(ptr0, len0, payload);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    exports.handle_dom_fast_path = handle_dom_fast_path;

    /**
     * @param {string} client_id
     * @param {any} payload
     * @returns {Promise<void>}
     */
    function handle_dom_webrtc_event(client_id, payload) {
        const ptr0 = passStringToWasm0(client_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.handle_dom_webrtc_event(ptr0, len0, payload);
        return ret;
    }
    exports.handle_dom_webrtc_event = handle_dom_webrtc_event;

    /**
     * WASM initialization entry point
     */
    function init() {
        wasm.init();
    }
    exports.init = init;

    function init_global() {
        const ret = wasm.init_global();
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    exports.init_global = init_global;

    /**
     * Register a new client (browser tab) with the SW runtime.
     *
     * Each call creates an independent runtime with its own signaling connection,
     * actor registration, and WebRTC state. This enables multiple browser tabs
     * to work simultaneously without interfering with each other.
     * @param {string} client_id
     * @param {any} config
     * @param {MessagePort} port
     * @returns {Promise<void>}
     */
    function register_client(client_id, config, port) {
        const ptr0 = passStringToWasm0(client_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.register_client(ptr0, len0, config, port);
        return ret;
    }
    exports.register_client = register_client;

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
     * @param {string} client_id
     * @param {string} peer_id
     * @param {MessagePort} port
     * @returns {Promise<void>}
     */
    function register_datachannel_port(client_id, peer_id, port) {
        const ptr0 = passStringToWasm0(client_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(peer_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.register_datachannel_port(ptr0, len0, ptr1, len1, port);
        return ret;
    }
    exports.register_datachannel_port = register_datachannel_port;

    /**
     * Register the local `SendEcho` service handler.
     *
     * The handler dispatches RPC requests to the concrete `SendEcho` methods:
     * - `echo.SendEcho.SendEcho`: discovers the remote `EchoService` and forwards the request
     */
    function register_echo_client_handler() {
        wasm.register_echo_client_handler();
    }
    exports.register_echo_client_handler = register_echo_client_handler;

    /**
     * Unregister a client (browser tab) from the SW runtime.
     *
     * Closes the signaling WebSocket (so the signaling server removes
     * the actor from its ServiceRegistry) and removes the client context.
     * Background tasks (signaling relay, heartbeat) will naturally stop
     * when the signaling connection drops.
     * @param {string} client_id
     * @returns {Promise<void>}
     */
    function unregister_client(client_id) {
        const ptr0 = passStringToWasm0(client_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.unregister_client(ptr0, len0);
        return ret;
    }
    exports.unregister_client = unregister_client;

    function __wbg_get_imports() {
        const import0 = {
            __proto__: null,
            __wbg_Error_83742b46f01ce22d: function(arg0, arg1) {
                const ret = Error(getStringFromWasm0(arg0, arg1));
                return ret;
            },
            __wbg_Number_a5a435bd7bbec835: function(arg0) {
                const ret = Number(arg0);
                return ret;
            },
            __wbg_String_8564e559799eccda: function(arg0, arg1) {
                const ret = String(arg1);
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg___wbindgen_bigint_get_as_i64_447a76b5c6ef7bda: function(arg0, arg1) {
                const v = arg1;
                const ret = typeof(v) === 'bigint' ? v : undefined;
                getDataViewMemory0().setBigInt64(arg0 + 8 * 1, isLikeNone(ret) ? BigInt(0) : ret, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
            },
            __wbg___wbindgen_boolean_get_c0f3f60bac5a78d1: function(arg0) {
                const v = arg0;
                const ret = typeof(v) === 'boolean' ? v : undefined;
                return isLikeNone(ret) ? 0xFFFFFF : ret ? 1 : 0;
            },
            __wbg___wbindgen_debug_string_5398f5bb970e0daa: function(arg0, arg1) {
                const ret = debugString(arg1);
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg___wbindgen_in_41dbb8413020e076: function(arg0, arg1) {
                const ret = arg0 in arg1;
                return ret;
            },
            __wbg___wbindgen_is_bigint_e2141d4f045b7eda: function(arg0) {
                const ret = typeof(arg0) === 'bigint';
                return ret;
            },
            __wbg___wbindgen_is_function_3c846841762788c1: function(arg0) {
                const ret = typeof(arg0) === 'function';
                return ret;
            },
            __wbg___wbindgen_is_null_0b605fc6b167c56f: function(arg0) {
                const ret = arg0 === null;
                return ret;
            },
            __wbg___wbindgen_is_object_781bc9f159099513: function(arg0) {
                const val = arg0;
                const ret = typeof(val) === 'object' && val !== null;
                return ret;
            },
            __wbg___wbindgen_is_string_7ef6b97b02428fae: function(arg0) {
                const ret = typeof(arg0) === 'string';
                return ret;
            },
            __wbg___wbindgen_is_undefined_52709e72fb9f179c: function(arg0) {
                const ret = arg0 === undefined;
                return ret;
            },
            __wbg___wbindgen_jsval_eq_ee31bfad3e536463: function(arg0, arg1) {
                const ret = arg0 === arg1;
                return ret;
            },
            __wbg___wbindgen_jsval_loose_eq_5bcc3bed3c69e72b: function(arg0, arg1) {
                const ret = arg0 == arg1;
                return ret;
            },
            __wbg___wbindgen_number_get_34bb9d9dcfa21373: function(arg0, arg1) {
                const obj = arg1;
                const ret = typeof(obj) === 'number' ? obj : undefined;
                getDataViewMemory0().setFloat64(arg0 + 8 * 1, isLikeNone(ret) ? 0 : ret, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
            },
            __wbg___wbindgen_string_get_395e606bd0ee4427: function(arg0, arg1) {
                const obj = arg1;
                const ret = typeof(obj) === 'string' ? obj : undefined;
                var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                var len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg___wbindgen_throw_6ddd609b62940d55: function(arg0, arg1) {
                throw new Error(getStringFromWasm0(arg0, arg1));
            },
            __wbg__wbg_cb_unref_6b5b6b8576d35cb1: function(arg0) {
                arg0._wbg_cb_unref();
            },
            __wbg_addEventListener_2d985aa8a656f6dc: function() { return handleError(function (arg0, arg1, arg2, arg3) {
                arg0.addEventListener(getStringFromWasm0(arg1, arg2), arg3);
            }, arguments); },
            __wbg_add_31c3a85003d5143e: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.add(arg1, arg2);
                return ret;
            }, arguments); },
            __wbg_add_7857847c343fb7de: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.add(arg1);
                return ret;
            }, arguments); },
            __wbg_call_2d781c1f4d5c0ef8: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.call(arg1, arg2);
                return ret;
            }, arguments); },
            __wbg_call_e133b57c9155d22c: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.call(arg1);
                return ret;
            }, arguments); },
            __wbg_clearTimeout_113b1cde814ec762: function(arg0) {
                const ret = clearTimeout(arg0);
                return ret;
            },
            __wbg_clear_1885f7bf35006b0c: function() { return handleError(function (arg0) {
                const ret = arg0.clear();
                return ret;
            }, arguments); },
            __wbg_close_af26905c832a88cb: function() { return handleError(function (arg0) {
                arg0.close();
            }, arguments); },
            __wbg_close_cbf870bdad0aad99: function(arg0) {
                arg0.close();
            },
            __wbg_code_aea376e2d265a64f: function(arg0) {
                const ret = arg0.code;
                return ret;
            },
            __wbg_createIndex_323cb0213cc21d9b: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4) {
                const ret = arg0.createIndex(getStringFromWasm0(arg1, arg2), arg3, arg4);
                return ret;
            }, arguments); },
            __wbg_createObjectStore_4709de9339ffc6c0: function() { return handleError(function (arg0, arg1, arg2, arg3) {
                const ret = arg0.createObjectStore(getStringFromWasm0(arg1, arg2), arg3);
                return ret;
            }, arguments); },
            __wbg_data_2e49deffd56e1b77: function(arg0) {
                const ret = arg0.data;
                return ret;
            },
            __wbg_data_a3d9ff9cdd801002: function(arg0) {
                const ret = arg0.data;
                return ret;
            },
            __wbg_debug_271c16e6de0bc226: function(arg0, arg1, arg2, arg3) {
                console.debug(arg0, arg1, arg2, arg3);
            },
            __wbg_deleteIndex_9391b8bace7b0b18: function() { return handleError(function (arg0, arg1, arg2) {
                arg0.deleteIndex(getStringFromWasm0(arg1, arg2));
            }, arguments); },
            __wbg_deleteObjectStore_65401ab024ac08c1: function() { return handleError(function (arg0, arg1, arg2) {
                arg0.deleteObjectStore(getStringFromWasm0(arg1, arg2));
            }, arguments); },
            __wbg_delete_40db93c05c546fb9: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.delete(arg1);
                return ret;
            }, arguments); },
            __wbg_done_08ce71ee07e3bd17: function(arg0) {
                const ret = arg0.done;
                return ret;
            },
            __wbg_entries_e8a20ff8c9757101: function(arg0) {
                const ret = Object.entries(arg0);
                return ret;
            },
            __wbg_error_1eece6b0039034ce: function(arg0, arg1, arg2, arg3) {
                console.error(arg0, arg1, arg2, arg3);
            },
            __wbg_error_57ef6dadfcb01843: function(arg0) {
                const ret = arg0.error;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_error_74898554122344a8: function() { return handleError(function (arg0) {
                const ret = arg0.error;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            }, arguments); },
            __wbg_error_8d9a8e04cd1d3588: function(arg0) {
                console.error(arg0);
            },
            __wbg_error_a6fa202b58aa1cd3: function(arg0, arg1) {
                let deferred0_0;
                let deferred0_1;
                try {
                    deferred0_0 = arg0;
                    deferred0_1 = arg1;
                    console.error(getStringFromWasm0(arg0, arg1));
                } finally {
                    wasm.__wbindgen_free(deferred0_0, deferred0_1, 1);
                }
            },
            __wbg_getAll_1c496368e98193a6: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.getAll(arg1, arg2 >>> 0);
                return ret;
            }, arguments); },
            __wbg_getAll_690f659b57ae2d51: function() { return handleError(function (arg0) {
                const ret = arg0.getAll();
                return ret;
            }, arguments); },
            __wbg_getAll_a959860fbb7a424a: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.getAll(arg1);
                return ret;
            }, arguments); },
            __wbg_getRandomValues_3dda8830c2565714: function() { return handleError(function (arg0, arg1) {
                globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
            }, arguments); },
            __wbg_get_326e41e095fb2575: function() { return handleError(function (arg0, arg1) {
                const ret = Reflect.get(arg0, arg1);
                return ret;
            }, arguments); },
            __wbg_get_3ef1eba1850ade27: function() { return handleError(function (arg0, arg1) {
                const ret = Reflect.get(arg0, arg1);
                return ret;
            }, arguments); },
            __wbg_get_7873e3afa59bad00: function(arg0, arg1, arg2) {
                const ret = arg1[arg2 >>> 0];
                var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                var len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_get_a8ee5c45dabc1b3b: function(arg0, arg1) {
                const ret = arg0[arg1 >>> 0];
                return ret;
            },
            __wbg_get_unchecked_329cfe50afab7352: function(arg0, arg1) {
                const ret = arg0[arg1 >>> 0];
                return ret;
            },
            __wbg_get_with_ref_key_6412cf3094599694: function(arg0, arg1) {
                const ret = arg0[arg1];
                return ret;
            },
            __wbg_indexNames_3a9be68017fb9405: function(arg0) {
                const ret = arg0.indexNames;
                return ret;
            },
            __wbg_index_f1b3b30c5d5af6fb: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.index(getStringFromWasm0(arg1, arg2));
                return ret;
            }, arguments); },
            __wbg_info_0194681687b5ab04: function(arg0, arg1, arg2, arg3) {
                console.info(arg0, arg1, arg2, arg3);
            },
            __wbg_instanceof_ArrayBuffer_101e2bf31071a9f6: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof ArrayBuffer;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbDatabase_5f436cc89cc07f14: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBDatabase;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbFactory_efcffbfd9020e4ac: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBFactory;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbOpenDbRequest_10c2576001eb6613: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBOpenDBRequest;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbRequest_6a0e24572d4f1d46: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBRequest;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbTransaction_125db5cfd1c1bfd2: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBTransaction;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_Map_f194b366846aca0c: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof Map;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_Object_be1962063fcc0c9f: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof Object;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_ServiceWorkerGlobalScope_f90f3fc36442975e: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof ServiceWorkerGlobalScope;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_Uint8Array_740438561a5b956d: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof Uint8Array;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_isArray_33b91feb269ff46e: function(arg0) {
                const ret = Array.isArray(arg0);
                return ret;
            },
            __wbg_isSafeInteger_ecd6a7f9c3e053cd: function(arg0) {
                const ret = Number.isSafeInteger(arg0);
                return ret;
            },
            __wbg_iterator_d8f549ec8fb061b1: function() {
                const ret = Symbol.iterator;
                return ret;
            },
            __wbg_keyPath_f17010debffed49a: function() { return handleError(function (arg0) {
                const ret = arg0.keyPath;
                return ret;
            }, arguments); },
            __wbg_length_02c4f6002306a824: function(arg0) {
                const ret = arg0.length;
                return ret;
            },
            __wbg_length_b3416cf66a5452c8: function(arg0) {
                const ret = arg0.length;
                return ret;
            },
            __wbg_length_ea16607d7b61445b: function(arg0) {
                const ret = arg0.length;
                return ret;
            },
            __wbg_log_524eedafa26daa59: function(arg0) {
                console.log(arg0);
            },
            __wbg_log_70972330cfc941dd: function(arg0, arg1, arg2, arg3) {
                console.log(arg0, arg1, arg2, arg3);
            },
            __wbg_multiEntry_fd907a11ddf44df1: function(arg0) {
                const ret = arg0.multiEntry;
                return ret;
            },
            __wbg_new_227d7c05414eb861: function() {
                const ret = new Error();
                return ret;
            },
            __wbg_new_5f486cdf45a04d78: function(arg0) {
                const ret = new Uint8Array(arg0);
                return ret;
            },
            __wbg_new_a70fbab9066b301f: function() {
                const ret = new Array();
                return ret;
            },
            __wbg_new_ab79df5bd7c26067: function() {
                const ret = new Object();
                return ret;
            },
            __wbg_new_dd50bcc3f60ba434: function() { return handleError(function (arg0, arg1) {
                const ret = new WebSocket(getStringFromWasm0(arg0, arg1));
                return ret;
            }, arguments); },
            __wbg_new_from_slice_22da9388ac046e50: function(arg0, arg1) {
                const ret = new Uint8Array(getArrayU8FromWasm0(arg0, arg1));
                return ret;
            },
            __wbg_new_typed_aaaeaf29cf802876: function(arg0, arg1) {
                try {
                    var state0 = {a: arg0, b: arg1};
                    var cb0 = (arg0, arg1) => {
                        const a = state0.a;
                        state0.a = 0;
                        try {
                            return wasm_bindgen__convert__closures_____invoke__h4fb9192c223e57b8(a, state0.b, arg0, arg1);
                        } finally {
                            state0.a = a;
                        }
                    };
                    const ret = new Promise(cb0);
                    return ret;
                } finally {
                    state0.a = state0.b = 0;
                }
            },
            __wbg_new_typed_bccac67128ed885a: function() {
                const ret = new Array();
                return ret;
            },
            __wbg_next_11b99ee6237339e3: function() { return handleError(function (arg0) {
                const ret = arg0.next();
                return ret;
            }, arguments); },
            __wbg_next_e01a967809d1aa68: function(arg0) {
                const ret = arg0.next;
                return ret;
            },
            __wbg_now_16f0c993d5dd6c27: function() {
                const ret = Date.now();
                return ret;
            },
            __wbg_objectStoreNames_564985d2e9ae7523: function(arg0) {
                const ret = arg0.objectStoreNames;
                return ret;
            },
            __wbg_objectStore_f314ab152a5c7bd0: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.objectStore(getStringFromWasm0(arg1, arg2));
                return ret;
            }, arguments); },
            __wbg_open_e7a9d3d6344572f6: function() { return handleError(function (arg0, arg1, arg2, arg3) {
                const ret = arg0.open(getStringFromWasm0(arg1, arg2), arg3 >>> 0);
                return ret;
            }, arguments); },
            __wbg_open_f3dc09caa3990bc4: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.open(getStringFromWasm0(arg1, arg2));
                return ret;
            }, arguments); },
            __wbg_postMessage_c89a8b5edbf59ad0: function() { return handleError(function (arg0, arg1) {
                arg0.postMessage(arg1);
            }, arguments); },
            __wbg_prototypesetcall_d62e5099504357e6: function(arg0, arg1, arg2) {
                Uint8Array.prototype.set.call(getArrayU8FromWasm0(arg0, arg1), arg2);
            },
            __wbg_push_e87b0e732085a946: function(arg0, arg1) {
                const ret = arg0.push(arg1);
                return ret;
            },
            __wbg_queueMicrotask_0c399741342fb10f: function(arg0) {
                const ret = arg0.queueMicrotask;
                return ret;
            },
            __wbg_queueMicrotask_a082d78ce798393e: function(arg0) {
                queueMicrotask(arg0);
            },
            __wbg_random_5bb86cae65a45bf6: function() {
                const ret = Math.random();
                return ret;
            },
            __wbg_readyState_1f1e7f1bdf9f4d42: function(arg0) {
                const ret = arg0.readyState;
                return ret;
            },
            __wbg_reason_cbcb9911796c4714: function(arg0, arg1) {
                const ret = arg1.reason;
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_resolve_ae8d83246e5bcc12: function(arg0) {
                const ret = Promise.resolve(arg0);
                return ret;
            },
            __wbg_result_c5baa2d3d690a01a: function() { return handleError(function (arg0) {
                const ret = arg0.result;
                return ret;
            }, arguments); },
            __wbg_send_d31a693c975dea74: function() { return handleError(function (arg0, arg1, arg2) {
                arg0.send(getArrayU8FromWasm0(arg1, arg2));
            }, arguments); },
            __wbg_setTimeout_ef24d2fc3ad97385: function() { return handleError(function (arg0, arg1) {
                const ret = setTimeout(arg0, arg1);
                return ret;
            }, arguments); },
            __wbg_set_282384002438957f: function(arg0, arg1, arg2) {
                arg0[arg1 >>> 0] = arg2;
            },
            __wbg_set_6be42768c690e380: function(arg0, arg1, arg2) {
                arg0[arg1] = arg2;
            },
            __wbg_set_7eaa4f96924fd6b3: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = Reflect.set(arg0, arg1, arg2);
                return ret;
            }, arguments); },
            __wbg_set_auto_increment_ffc3cd6470763a4c: function(arg0, arg1) {
                arg0.autoIncrement = arg1 !== 0;
            },
            __wbg_set_binaryType_3dcf8281ec100a8f: function(arg0, arg1) {
                arg0.binaryType = __wbindgen_enum_BinaryType[arg1];
            },
            __wbg_set_key_path_3c45a8ff0b89e678: function(arg0, arg1) {
                arg0.keyPath = arg1;
            },
            __wbg_set_multi_entry_38c253febe05d3be: function(arg0, arg1) {
                arg0.multiEntry = arg1 !== 0;
            },
            __wbg_set_name_02d633afec2e2bf0: function(arg0, arg1, arg2) {
                arg0.name = getStringFromWasm0(arg1, arg2);
            },
            __wbg_set_onabort_63885d8d7841a8d5: function(arg0, arg1) {
                arg0.onabort = arg1;
            },
            __wbg_set_onclose_8da801226bdd7a7b: function(arg0, arg1) {
                arg0.onclose = arg1;
            },
            __wbg_set_oncomplete_f31e6dc6d16c1ff8: function(arg0, arg1) {
                arg0.oncomplete = arg1;
            },
            __wbg_set_onerror_8a268cb237177bba: function(arg0, arg1) {
                arg0.onerror = arg1;
            },
            __wbg_set_onerror_901ca711f94a5bbb: function(arg0, arg1) {
                arg0.onerror = arg1;
            },
            __wbg_set_onerror_c1ecd6233c533c08: function(arg0, arg1) {
                arg0.onerror = arg1;
            },
            __wbg_set_onmessage_6f80ab771bf151aa: function(arg0, arg1) {
                arg0.onmessage = arg1;
            },
            __wbg_set_onopen_34e3e24cf9337ddd: function(arg0, arg1) {
                arg0.onopen = arg1;
            },
            __wbg_set_onsuccess_fca94ded107b64af: function(arg0, arg1) {
                arg0.onsuccess = arg1;
            },
            __wbg_set_onupgradeneeded_860ce42184f987e7: function(arg0, arg1) {
                arg0.onupgradeneeded = arg1;
            },
            __wbg_set_onversionchange_3d88930f82c97b92: function(arg0, arg1) {
                arg0.onversionchange = arg1;
            },
            __wbg_set_unique_a39d85db47f8e025: function(arg0, arg1) {
                arg0.unique = arg1 !== 0;
            },
            __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
                const ret = arg1.stack;
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_static_accessor_GLOBAL_8adb955bd33fac2f: function() {
                const ret = typeof global === 'undefined' ? null : global;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_static_accessor_GLOBAL_THIS_ad356e0db91c7913: function() {
                const ret = typeof globalThis === 'undefined' ? null : globalThis;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_static_accessor_SELF_f207c857566db248: function() {
                const ret = typeof self === 'undefined' ? null : self;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_static_accessor_WINDOW_bb9f1ba69d61b386: function() {
                const ret = typeof window === 'undefined' ? null : window;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_target_7bc90f314634b37b: function(arg0) {
                const ret = arg0.target;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_then_098abe61755d12f6: function(arg0, arg1) {
                const ret = arg0.then(arg1);
                return ret;
            },
            __wbg_transaction_3223f7c8d0f40129: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.transaction(arg1, __wbindgen_enum_IdbTransactionMode[arg2]);
                return ret;
            }, arguments); },
            __wbg_transaction_fda57653957fee06: function(arg0) {
                const ret = arg0.transaction;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_unique_3329c63c37e586a7: function(arg0) {
                const ret = arg0.unique;
                return ret;
            },
            __wbg_url_778f9516ea867e17: function(arg0, arg1) {
                const ret = arg1.url;
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_value_21fc78aab0322612: function(arg0) {
                const ret = arg0.value;
                return ret;
            },
            __wbg_warn_809cad1bfc2b3a42: function(arg0, arg1, arg2, arg3) {
                console.warn(arg0, arg1, arg2, arg3);
            },
            __wbg_wasClean_69f68dc4ed2d2cc7: function(arg0) {
                const ret = arg0.wasClean;
                return ret;
            },
            __wbindgen_cast_0000000000000001: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 164, function: Function { arguments: [Externref], shim_idx: 165, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h5a05f2a88d4b9300, wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3);
                return ret;
            },
            __wbindgen_cast_0000000000000002: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 164, function: Function { arguments: [NamedExternref("CloseEvent")], shim_idx: 165, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h5a05f2a88d4b9300, wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_1);
                return ret;
            },
            __wbindgen_cast_0000000000000003: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 164, function: Function { arguments: [NamedExternref("ExtendableMessageEvent")], shim_idx: 165, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h5a05f2a88d4b9300, wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_2);
                return ret;
            },
            __wbindgen_cast_0000000000000004: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 164, function: Function { arguments: [NamedExternref("IDBVersionChangeEvent")], shim_idx: 165, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h5a05f2a88d4b9300, wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_3);
                return ret;
            },
            __wbindgen_cast_0000000000000005: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 164, function: Function { arguments: [NamedExternref("MessageEvent")], shim_idx: 165, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h5a05f2a88d4b9300, wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_4);
                return ret;
            },
            __wbindgen_cast_0000000000000006: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 276, function: Function { arguments: [], shim_idx: 277, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__hff5f1365318bebd6, wasm_bindgen__convert__closures_____invoke__h9584e8edcc7fcc5d);
                return ret;
            },
            __wbindgen_cast_0000000000000007: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 290, function: Function { arguments: [Externref], shim_idx: 291, ret: Result(Unit), inner_ret: Some(Result(Unit)) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h329e548a2f539608, wasm_bindgen__convert__closures_____invoke__hf5ee9d21c8541ba9);
                return ret;
            },
            __wbindgen_cast_0000000000000008: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 361, function: Function { arguments: [NamedExternref("Event")], shim_idx: 362, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h2bf42a8b2ffdb9c9, wasm_bindgen__convert__closures_____invoke__h4b2b0d0c520d2fe0);
                return ret;
            },
            __wbindgen_cast_0000000000000009: function(arg0) {
                // Cast intrinsic for `F64 -> Externref`.
                const ret = arg0;
                return ret;
            },
            __wbindgen_cast_000000000000000a: function(arg0) {
                // Cast intrinsic for `I64 -> Externref`.
                const ret = arg0;
                return ret;
            },
            __wbindgen_cast_000000000000000b: function(arg0, arg1) {
                // Cast intrinsic for `Ref(Slice(U8)) -> NamedExternref("Uint8Array")`.
                const ret = getArrayU8FromWasm0(arg0, arg1);
                return ret;
            },
            __wbindgen_cast_000000000000000c: function(arg0, arg1) {
                // Cast intrinsic for `Ref(String) -> Externref`.
                const ret = getStringFromWasm0(arg0, arg1);
                return ret;
            },
            __wbindgen_cast_000000000000000d: function(arg0) {
                // Cast intrinsic for `U64 -> Externref`.
                const ret = BigInt.asUintN(64, arg0);
                return ret;
            },
            __wbindgen_init_externref_table: function() {
                const table = wasm.__wbindgen_externrefs;
                const offset = table.grow(4);
                table.set(0, undefined);
                table.set(offset + 0, undefined);
                table.set(offset + 1, null);
                table.set(offset + 2, true);
                table.set(offset + 3, false);
            },
        };
        return {
            __proto__: null,
            "./echo_client_bg.js": import0,
        };
    }

    function wasm_bindgen__convert__closures_____invoke__h9584e8edcc7fcc5d(arg0, arg1) {
        wasm.wasm_bindgen__convert__closures_____invoke__h9584e8edcc7fcc5d(arg0, arg1);
    }

    function wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_1(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_1(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_2(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_2(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_3(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_3(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_4(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h263251508bdf13c3_4(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__h4b2b0d0c520d2fe0(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h4b2b0d0c520d2fe0(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__hf5ee9d21c8541ba9(arg0, arg1, arg2) {
        const ret = wasm.wasm_bindgen__convert__closures_____invoke__hf5ee9d21c8541ba9(arg0, arg1, arg2);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }

    function wasm_bindgen__convert__closures_____invoke__h4fb9192c223e57b8(arg0, arg1, arg2, arg3) {
        wasm.wasm_bindgen__convert__closures_____invoke__h4fb9192c223e57b8(arg0, arg1, arg2, arg3);
    }


    const __wbindgen_enum_BinaryType = ["blob", "arraybuffer"];


    const __wbindgen_enum_IdbTransactionMode = ["readonly", "readwrite", "versionchange", "readwriteflush", "cleanup"];

    function addToExternrefTable0(obj) {
        const idx = wasm.__externref_table_alloc();
        wasm.__wbindgen_externrefs.set(idx, obj);
        return idx;
    }

    const CLOSURE_DTORS = (typeof FinalizationRegistry === 'undefined')
        ? { register: () => {}, unregister: () => {} }
        : new FinalizationRegistry(state => state.dtor(state.a, state.b));

    function debugString(val) {
        // primitive types
        const type = typeof val;
        if (type == 'number' || type == 'boolean' || val == null) {
            return  `${val}`;
        }
        if (type == 'string') {
            return `"${val}"`;
        }
        if (type == 'symbol') {
            const description = val.description;
            if (description == null) {
                return 'Symbol';
            } else {
                return `Symbol(${description})`;
            }
        }
        if (type == 'function') {
            const name = val.name;
            if (typeof name == 'string' && name.length > 0) {
                return `Function(${name})`;
            } else {
                return 'Function';
            }
        }
        // objects
        if (Array.isArray(val)) {
            const length = val.length;
            let debug = '[';
            if (length > 0) {
                debug += debugString(val[0]);
            }
            for(let i = 1; i < length; i++) {
                debug += ', ' + debugString(val[i]);
            }
            debug += ']';
            return debug;
        }
        // Test for built-in
        const builtInMatches = /\[object ([^\]]+)\]/.exec(toString.call(val));
        let className;
        if (builtInMatches && builtInMatches.length > 1) {
            className = builtInMatches[1];
        } else {
            // Failed to match the standard '[object ClassName]'
            return toString.call(val);
        }
        if (className == 'Object') {
            // we're a user defined class or Object
            // JSON.stringify avoids problems with cycles, and is generally much
            // easier than looping through ownProperties of `val`.
            try {
                return 'Object(' + JSON.stringify(val) + ')';
            } catch (_) {
                return 'Object';
            }
        }
        // errors
        if (val instanceof Error) {
            return `${val.name}: ${val.message}\n${val.stack}`;
        }
        // TODO we could test for more things here, like `Set`s and `Map`s.
        return className;
    }

    function getArrayU8FromWasm0(ptr, len) {
        ptr = ptr >>> 0;
        return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
    }

    let cachedDataViewMemory0 = null;
    function getDataViewMemory0() {
        if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
            cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
        }
        return cachedDataViewMemory0;
    }

    function getStringFromWasm0(ptr, len) {
        ptr = ptr >>> 0;
        return decodeText(ptr, len);
    }

    let cachedUint8ArrayMemory0 = null;
    function getUint8ArrayMemory0() {
        if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
            cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
        }
        return cachedUint8ArrayMemory0;
    }

    function handleError(f, args) {
        try {
            return f.apply(this, args);
        } catch (e) {
            const idx = addToExternrefTable0(e);
            wasm.__wbindgen_exn_store(idx);
        }
    }

    function isLikeNone(x) {
        return x === undefined || x === null;
    }

    function makeMutClosure(arg0, arg1, dtor, f) {
        const state = { a: arg0, b: arg1, cnt: 1, dtor };
        const real = (...args) => {

            // First up with a closure we increment the internal reference
            // count. This ensures that the Rust closure environment won't
            // be deallocated while we're invoking it.
            state.cnt++;
            const a = state.a;
            state.a = 0;
            try {
                return f(a, state.b, ...args);
            } finally {
                state.a = a;
                real._wbg_cb_unref();
            }
        };
        real._wbg_cb_unref = () => {
            if (--state.cnt === 0) {
                state.dtor(state.a, state.b);
                state.a = 0;
                CLOSURE_DTORS.unregister(state);
            }
        };
        CLOSURE_DTORS.register(real, state, state);
        return real;
    }

    function passStringToWasm0(arg, malloc, realloc) {
        if (realloc === undefined) {
            const buf = cachedTextEncoder.encode(arg);
            const ptr = malloc(buf.length, 1) >>> 0;
            getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
            WASM_VECTOR_LEN = buf.length;
            return ptr;
        }

        let len = arg.length;
        let ptr = malloc(len, 1) >>> 0;

        const mem = getUint8ArrayMemory0();

        let offset = 0;

        for (; offset < len; offset++) {
            const code = arg.charCodeAt(offset);
            if (code > 0x7F) break;
            mem[ptr + offset] = code;
        }
        if (offset !== len) {
            if (offset !== 0) {
                arg = arg.slice(offset);
            }
            ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
            const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
            const ret = cachedTextEncoder.encodeInto(arg, view);

            offset += ret.written;
            ptr = realloc(ptr, len, offset, 1) >>> 0;
        }

        WASM_VECTOR_LEN = offset;
        return ptr;
    }

    function takeFromExternrefTable0(idx) {
        const value = wasm.__wbindgen_externrefs.get(idx);
        wasm.__externref_table_dealloc(idx);
        return value;
    }

    let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
    cachedTextDecoder.decode();
    function decodeText(ptr, len) {
        return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
    }

    const cachedTextEncoder = new TextEncoder();

    if (!('encodeInto' in cachedTextEncoder)) {
        cachedTextEncoder.encodeInto = function (arg, view) {
            const buf = cachedTextEncoder.encode(arg);
            view.set(buf);
            return {
                read: arg.length,
                written: buf.length
            };
        };
    }

    let WASM_VECTOR_LEN = 0;

    let wasmModule, wasm;
    function __wbg_finalize_init(instance, module) {
        wasm = instance.exports;
        wasmModule = module;
        cachedDataViewMemory0 = null;
        cachedUint8ArrayMemory0 = null;
        wasm.__wbindgen_start();
        return wasm;
    }

    async function __wbg_load(module, imports) {
        if (typeof Response === 'function' && module instanceof Response) {
            if (typeof WebAssembly.instantiateStreaming === 'function') {
                try {
                    return await WebAssembly.instantiateStreaming(module, imports);
                } catch (e) {
                    const validResponse = module.ok && expectedResponseType(module.type);

                    if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                        console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                    } else { throw e; }
                }
            }

            const bytes = await module.arrayBuffer();
            return await WebAssembly.instantiate(bytes, imports);
        } else {
            const instance = await WebAssembly.instantiate(module, imports);

            if (instance instanceof WebAssembly.Instance) {
                return { instance, module };
            } else {
                return instance;
            }
        }

        function expectedResponseType(type) {
            switch (type) {
                case 'basic': case 'cors': case 'default': return true;
            }
            return false;
        }
    }

    function initSync(module) {
        if (wasm !== undefined) return wasm;


        if (module !== undefined) {
            if (Object.getPrototypeOf(module) === Object.prototype) {
                ({module} = module)
            } else {
                console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
            }
        }

        const imports = __wbg_get_imports();
        if (!(module instanceof WebAssembly.Module)) {
            module = new WebAssembly.Module(module);
        }
        const instance = new WebAssembly.Instance(module, imports);
        return __wbg_finalize_init(instance, module);
    }

    async function __wbg_init(module_or_path) {
        if (wasm !== undefined) return wasm;


        if (module_or_path !== undefined) {
            if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
                ({module_or_path} = module_or_path)
            } else {
                console.warn('using deprecated parameters for the initialization function; pass a single object instead')
            }
        }

        if (module_or_path === undefined && script_src !== undefined) {
            module_or_path = script_src.replace(/\.js$/, "_bg.wasm");
        }
        const imports = __wbg_get_imports();

        if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
            module_or_path = fetch(module_or_path);
        }

        const { instance, module } = await __wbg_load(await module_or_path, imports);

        return __wbg_finalize_init(instance, module);
    }

    return Object.assign(__wbg_init, { initSync }, exports);
})({ __proto__: null });
