let wasm_bindgen = (function(exports) {
    let script_src;
    if (typeof document !== 'undefined' && document.currentScript !== null) {
        script_src = new URL(document.currentScript.src, location.href).toString();
    }

    /**
     * Encode an `InitPayloadV1` for guest WASM initialization.
     *
     * Returns protobuf-encoded bytes that can be passed to the guest's `actr_init`.
     * @param {string} actr_type
     * @param {number} realm_id
     * @returns {Uint8Array}
     */
    function encode_guest_init_payload(actr_type, realm_id) {
        const ptr0 = passStringToWasm0(actr_type, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.encode_guest_init_payload(ptr0, len0, realm_id);
        var v2 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v2;
    }
    exports.encode_guest_init_payload = encode_guest_init_payload;

    /**
     * Handle a guest's outbound host invocation asynchronously.
     *
     * Called from JS when the guest WASM invokes `actr_host_invoke`.
     * The current `RuntimeContext` must be available in `GUEST_CTX`
     * (set by `register_guest_workload` during dispatch).
     *
     * Supports:
     * - `HOST_DISCOVER` (op=4): discover a target actor by `ActrType`
     * - `HOST_CALL_RAW` (op=3): raw RPC call to a target actor
     * - `HOST_CALL` (op=1): typed RPC call to a destination
     *
     * Returns protobuf-encoded `AbiReply` bytes.
     * @param {Uint8Array} frame_bytes
     * @returns {Promise<Uint8Array>}
     */
    function guest_host_invoke_async(frame_bytes) {
        const ptr0 = passArray8ToWasm0(frame_bytes, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.guest_host_invoke_async(ptr0, len0);
        return ret;
    }
    exports.guest_host_invoke_async = guest_host_invoke_async;

    /**
     * Handle an RPC control request originating from the DOM side.
     *
     * Message flow in unified-dispatcher mode:
     * - With `WORKLOAD`: `DOM -> workload.dispatch(route_key, payload, ctx) -> response`
     *   - Local route: the workload processes locally and may call remote targets via `ctx.call_raw()`
     *   - Remote route: the workload forwards to a remote actor via `ctx.call_raw()`
     * - Without `WORKLOAD`: `DOM -> HostGate -> Gate -> WebRTC` (legacy compatibility path)
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
     * Register a guest workload backed by a JS dispatch function.
     *
     * `dispatch_fn` signature (JS):
     *   `(abiFrameBytes: Uint8Array) => Uint8Array | Promise<Uint8Array>`
     *
     *   - **Input**: protobuf-encoded `AbiFrame` with `op = GUEST_HANDLE`
     *   - **Output**: protobuf-encoded `AbiReply` (sync or async via JSPI)
     *
     * This enables the SW runtime to dispatch RPC requests to a standard
     * guest WASM (built with `entry!` macro) loaded separately via
     * `WebAssembly.instantiate`.
     * @param {Function} dispatch_fn
     */
    function register_guest_workload(dispatch_fn) {
        wasm.register_guest_workload(dispatch_fn);
    }
    exports.register_guest_workload = register_guest_workload;

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
            __wbg_Error_55538483de6e3abe: function(arg0, arg1) {
                const ret = Error(getStringFromWasm0(arg0, arg1));
                return ret;
            },
            __wbg_Number_f257194b7002d6f9: function(arg0) {
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
            __wbg___wbindgen_bigint_get_as_i64_a738e80c0fe6f6a7: function(arg0, arg1) {
                const v = arg1;
                const ret = typeof(v) === 'bigint' ? v : undefined;
                getDataViewMemory0().setBigInt64(arg0 + 8 * 1, isLikeNone(ret) ? BigInt(0) : ret, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
            },
            __wbg___wbindgen_boolean_get_fe2a24fdfdb4064f: function(arg0) {
                const v = arg0;
                const ret = typeof(v) === 'boolean' ? v : undefined;
                return isLikeNone(ret) ? 0xFFFFFF : ret ? 1 : 0;
            },
            __wbg___wbindgen_debug_string_d89627202d0155b7: function(arg0, arg1) {
                const ret = debugString(arg1);
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg___wbindgen_in_fe3eb6a509f75744: function(arg0, arg1) {
                const ret = arg0 in arg1;
                return ret;
            },
            __wbg___wbindgen_is_bigint_ca270ac12ef71091: function(arg0) {
                const ret = typeof(arg0) === 'bigint';
                return ret;
            },
            __wbg___wbindgen_is_function_2a95406423ea8626: function(arg0) {
                const ret = typeof(arg0) === 'function';
                return ret;
            },
            __wbg___wbindgen_is_null_8d90524c9e0af183: function(arg0) {
                const ret = arg0 === null;
                return ret;
            },
            __wbg___wbindgen_is_object_59a002e76b059312: function(arg0) {
                const val = arg0;
                const ret = typeof(val) === 'object' && val !== null;
                return ret;
            },
            __wbg___wbindgen_is_string_624d5244bb2bc87c: function(arg0) {
                const ret = typeof(arg0) === 'string';
                return ret;
            },
            __wbg___wbindgen_is_undefined_87a3a837f331fef5: function(arg0) {
                const ret = arg0 === undefined;
                return ret;
            },
            __wbg___wbindgen_jsval_eq_eedd705f9f2a4f35: function(arg0, arg1) {
                const ret = arg0 === arg1;
                return ret;
            },
            __wbg___wbindgen_jsval_loose_eq_cf851f110c48f9ba: function(arg0, arg1) {
                const ret = arg0 == arg1;
                return ret;
            },
            __wbg___wbindgen_number_get_769f3676dc20c1d7: function(arg0, arg1) {
                const obj = arg1;
                const ret = typeof(obj) === 'number' ? obj : undefined;
                getDataViewMemory0().setFloat64(arg0 + 8 * 1, isLikeNone(ret) ? 0 : ret, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
            },
            __wbg___wbindgen_string_get_f1161390414f9b59: function(arg0, arg1) {
                const obj = arg1;
                const ret = typeof(obj) === 'string' ? obj : undefined;
                var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                var len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg___wbindgen_throw_5549492daedad139: function(arg0, arg1) {
                throw new Error(getStringFromWasm0(arg0, arg1));
            },
            __wbg__wbg_cb_unref_fbe69bb076c16bad: function(arg0) {
                arg0._wbg_cb_unref();
            },
            __wbg_abort_bdf419e9dcbdaeb3: function(arg0) {
                arg0.abort();
            },
            __wbg_addEventListener_ee34fcb181ae85b2: function() { return handleError(function (arg0, arg1, arg2, arg3) {
                arg0.addEventListener(getStringFromWasm0(arg1, arg2), arg3);
            }, arguments); },
            __wbg_add_0994d402d4852477: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.add(arg1, arg2);
                return ret;
            }, arguments); },
            __wbg_add_2b75f090867cc8d0: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.add(arg1);
                return ret;
            }, arguments); },
            __wbg_arrayBuffer_9f258d017f7107c5: function() { return handleError(function (arg0) {
                const ret = arg0.arrayBuffer();
                return ret;
            }, arguments); },
            __wbg_call_4f2f92601568b772: function() { return handleError(function (arg0, arg1, arg2, arg3) {
                const ret = arg0.call(arg1, arg2, arg3);
                return ret;
            }, arguments); },
            __wbg_call_6ae20895a60069a2: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.call(arg1);
                return ret;
            }, arguments); },
            __wbg_call_8f5d7bb070283508: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.call(arg1, arg2);
                return ret;
            }, arguments); },
            __wbg_clearTimeout_113b1cde814ec762: function(arg0) {
                const ret = clearTimeout(arg0);
                return ret;
            },
            __wbg_clear_c9410efdc2dbc8e0: function() { return handleError(function (arg0) {
                const ret = arg0.clear();
                return ret;
            }, arguments); },
            __wbg_close_1bf0654059764e94: function() { return handleError(function (arg0) {
                arg0.close();
            }, arguments); },
            __wbg_close_28e71c252a91bf2b: function(arg0) {
                arg0.close();
            },
            __wbg_code_7eb5b8af0cea9f25: function(arg0) {
                const ret = arg0.code;
                return ret;
            },
            __wbg_createIndex_de61f2bbba6841ed: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4) {
                const ret = arg0.createIndex(getStringFromWasm0(arg1, arg2), arg3, arg4);
                return ret;
            }, arguments); },
            __wbg_createObjectStore_19cd505be19da257: function() { return handleError(function (arg0, arg1, arg2, arg3) {
                const ret = arg0.createObjectStore(getStringFromWasm0(arg1, arg2), arg3);
                return ret;
            }, arguments); },
            __wbg_data_22be47be234e1ccc: function(arg0) {
                const ret = arg0.data;
                return ret;
            },
            __wbg_data_7de671a92a650aba: function(arg0) {
                const ret = arg0.data;
                return ret;
            },
            __wbg_debug_61e14ffba79c6807: function(arg0, arg1, arg2, arg3) {
                console.debug(arg0, arg1, arg2, arg3);
            },
            __wbg_deleteIndex_494dbfc4da08d56a: function() { return handleError(function (arg0, arg1, arg2) {
                arg0.deleteIndex(getStringFromWasm0(arg1, arg2));
            }, arguments); },
            __wbg_deleteObjectStore_ba944bf5d1131542: function() { return handleError(function (arg0, arg1, arg2) {
                arg0.deleteObjectStore(getStringFromWasm0(arg1, arg2));
            }, arguments); },
            __wbg_delete_1f77ffb307a45685: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.delete(arg1);
                return ret;
            }, arguments); },
            __wbg_done_19f92cb1f8738aba: function(arg0) {
                const ret = arg0.done;
                return ret;
            },
            __wbg_encodeURIComponent_eb884a8ffd374587: function(arg0, arg1) {
                const ret = encodeURIComponent(getStringFromWasm0(arg0, arg1));
                return ret;
            },
            __wbg_entries_28ed7cb892e12eff: function(arg0) {
                const ret = Object.entries(arg0);
                return ret;
            },
            __wbg_error_7a4215bdabbb32fa: function() { return handleError(function (arg0) {
                const ret = arg0.error;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            }, arguments); },
            __wbg_error_9126ccb9e84e16ff: function(arg0, arg1, arg2, arg3) {
                console.error(arg0, arg1, arg2, arg3);
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
            __wbg_error_de6b86e598505246: function(arg0) {
                console.error(arg0);
            },
            __wbg_error_e81d3bef98c39301: function(arg0) {
                const ret = arg0.error;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_fetch_3f39346b50886803: function(arg0, arg1) {
                const ret = arg0.fetch(arg1);
                return ret;
            },
            __wbg_getAll_610af591aba64e52: function() { return handleError(function (arg0) {
                const ret = arg0.getAll();
                return ret;
            }, arguments); },
            __wbg_getAll_ad9ee30b0d16b731: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.getAll(arg1, arg2 >>> 0);
                return ret;
            }, arguments); },
            __wbg_getAll_e57a65cdce5aa427: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.getAll(arg1);
                return ret;
            }, arguments); },
            __wbg_getKey_8e8341ac9a60adc4: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.getKey(arg1);
                return ret;
            }, arguments); },
            __wbg_getRandomValues_d49329ff89a07af1: function() { return handleError(function (arg0, arg1) {
                globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
            }, arguments); },
            __wbg_get_0d9dce10096bb060: function(arg0, arg1, arg2) {
                const ret = arg1[arg2 >>> 0];
                var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                var len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_get_94f5fc088edd3138: function(arg0, arg1) {
                const ret = arg0[arg1 >>> 0];
                return ret;
            },
            __wbg_get_a50328e7325d7f9b: function() { return handleError(function (arg0, arg1) {
                const ret = Reflect.get(arg0, arg1);
                return ret;
            }, arguments); },
            __wbg_get_fb70b26de64b403c: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.get(arg1);
                return ret;
            }, arguments); },
            __wbg_get_ff5f1fb220233477: function() { return handleError(function (arg0, arg1) {
                const ret = Reflect.get(arg0, arg1);
                return ret;
            }, arguments); },
            __wbg_get_unchecked_7c6bbabf5b0b1fbf: function(arg0, arg1) {
                const ret = arg0[arg1 >>> 0];
                return ret;
            },
            __wbg_get_with_ref_key_6412cf3094599694: function(arg0, arg1) {
                const ret = arg0[arg1];
                return ret;
            },
            __wbg_indexNames_43922756971132c9: function(arg0) {
                const ret = arg0.indexNames;
                return ret;
            },
            __wbg_index_f1966e77ef1ae8a8: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.index(getStringFromWasm0(arg1, arg2));
                return ret;
            }, arguments); },
            __wbg_info_4a899082bb96facf: function(arg0, arg1, arg2, arg3) {
                console.info(arg0, arg1, arg2, arg3);
            },
            __wbg_instanceof_ArrayBuffer_8d855993947fc3a2: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof ArrayBuffer;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbDatabase_00de432fa618564a: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBDatabase;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbFactory_19c4fa7e0782bc89: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBFactory;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbOpenDbRequest_3b7d7bf90aebc039: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBOpenDBRequest;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbRequest_855e13698c9d3f99: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBRequest;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbTransaction_06494035fdc2b674: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBTransaction;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_Map_238410f1463c05ed: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof Map;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_Object_d622a5764f4f9002: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof Object;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_Promise_bd142f1951b917ba: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof Promise;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_Response_fece7eabbcaca4c3: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof Response;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_ServiceWorkerGlobalScope_276d7f830aa044c7: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof ServiceWorkerGlobalScope;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_Uint8Array_ce24d58a5f4bdcc3: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof Uint8Array;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_isArray_867202cf8f195ed8: function(arg0) {
                const ret = Array.isArray(arg0);
                return ret;
            },
            __wbg_isSafeInteger_1dfae065cbfe1915: function(arg0) {
                const ret = Number.isSafeInteger(arg0);
                return ret;
            },
            __wbg_iterator_54661826e186eb6a: function() {
                const ret = Symbol.iterator;
                return ret;
            },
            __wbg_keyPath_721b23317a6fc5bd: function() { return handleError(function (arg0) {
                const ret = arg0.keyPath;
                return ret;
            }, arguments); },
            __wbg_length_6edd960dac4a5695: function(arg0) {
                const ret = arg0.length;
                return ret;
            },
            __wbg_length_e6e1633fbea6cfa9: function(arg0) {
                const ret = arg0.length;
                return ret;
            },
            __wbg_length_fae3e439140f48a4: function(arg0) {
                const ret = arg0.length;
                return ret;
            },
            __wbg_log_6a75b71d6316e935: function(arg0) {
                console.log(arg0);
            },
            __wbg_log_da87efab6aec47d3: function(arg0, arg1, arg2, arg3) {
                console.log(arg0, arg1, arg2, arg3);
            },
            __wbg_multiEntry_17b4232b0a018934: function(arg0) {
                const ret = arg0.multiEntry;
                return ret;
            },
            __wbg_new_1d96678aaacca32e: function(arg0) {
                const ret = new Uint8Array(arg0);
                return ret;
            },
            __wbg_new_210ef5849ab6cf48: function() { return handleError(function () {
                const ret = new Headers();
                return ret;
            }, arguments); },
            __wbg_new_227d7c05414eb861: function() {
                const ret = new Error();
                return ret;
            },
            __wbg_new_4370be21fa2b2f80: function() {
                const ret = new Array();
                return ret;
            },
            __wbg_new_48e1d86cfd30c8e7: function() {
                const ret = new Object();
                return ret;
            },
            __wbg_new_69642b0f6c3151cc: function() { return handleError(function (arg0, arg1) {
                const ret = new WebSocket(getStringFromWasm0(arg0, arg1));
                return ret;
            }, arguments); },
            __wbg_new_ce17f0bcfcc7b8ef: function() { return handleError(function () {
                const ret = new AbortController();
                return ret;
            }, arguments); },
            __wbg_new_from_slice_0bc58e36f82a1b50: function(arg0, arg1) {
                const ret = new Uint8Array(getArrayU8FromWasm0(arg0, arg1));
                return ret;
            },
            __wbg_new_typed_25dda2388d7e5e9f: function(arg0, arg1) {
                try {
                    var state0 = {a: arg0, b: arg1};
                    var cb0 = (arg0, arg1) => {
                        const a = state0.a;
                        state0.a = 0;
                        try {
                            return wasm_bindgen__convert__closures_____invoke__h3ed608c0d4f5ac52(a, state0.b, arg0, arg1);
                        } finally {
                            state0.a = a;
                        }
                    };
                    const ret = new Promise(cb0);
                    return ret;
                } finally {
                    state0.a = 0;
                }
            },
            __wbg_new_with_length_0f3108b57e05ed7c: function(arg0) {
                const ret = new Uint8Array(arg0 >>> 0);
                return ret;
            },
            __wbg_new_with_str_and_init_cb3df438bf62964e: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = new Request(getStringFromWasm0(arg0, arg1), arg2);
                return ret;
            }, arguments); },
            __wbg_next_55d835fe0ab5b3e7: function(arg0) {
                const ret = arg0.next;
                return ret;
            },
            __wbg_next_e34cfb9df1518d7c: function() { return handleError(function (arg0) {
                const ret = arg0.next();
                return ret;
            }, arguments); },
            __wbg_now_46736a527d2e74e7: function() {
                const ret = Date.now();
                return ret;
            },
            __wbg_objectStoreNames_a0efd78a246af0de: function(arg0) {
                const ret = arg0.objectStoreNames;
                return ret;
            },
            __wbg_objectStore_37f98f8f6e547d16: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.objectStore(getStringFromWasm0(arg1, arg2));
                return ret;
            }, arguments); },
            __wbg_open_e6bfc207e91cd2ce: function() { return handleError(function (arg0, arg1, arg2, arg3) {
                const ret = arg0.open(getStringFromWasm0(arg1, arg2), arg3 >>> 0);
                return ret;
            }, arguments); },
            __wbg_open_efce8e3719a25f1f: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.open(getStringFromWasm0(arg1, arg2));
                return ret;
            }, arguments); },
            __wbg_postMessage_e5dce4dcd1f8f2bf: function() { return handleError(function (arg0, arg1) {
                arg0.postMessage(arg1);
            }, arguments); },
            __wbg_prototypesetcall_3875d54d12ef2eec: function(arg0, arg1, arg2) {
                Uint8Array.prototype.set.call(getArrayU8FromWasm0(arg0, arg1), arg2);
            },
            __wbg_push_d0006a37f9fcda6d: function(arg0, arg1) {
                const ret = arg0.push(arg1);
                return ret;
            },
            __wbg_put_320f06a5d11cb50b: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.put(arg1, arg2);
                return ret;
            }, arguments); },
            __wbg_put_b00d93ec132c6bb3: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.put(arg1);
                return ret;
            }, arguments); },
            __wbg_queueMicrotask_8868365114fe23b5: function(arg0) {
                queueMicrotask(arg0);
            },
            __wbg_queueMicrotask_cfc5a0e62f9ebdbe: function(arg0) {
                const ret = arg0.queueMicrotask;
                return ret;
            },
            __wbg_random_09b0bd71e83551d7: function() {
                const ret = Math.random();
                return ret;
            },
            __wbg_readyState_a08d25cc57214030: function(arg0) {
                const ret = arg0.readyState;
                return ret;
            },
            __wbg_reason_30c85ca866e286f0: function(arg0, arg1) {
                const ret = arg1.reason;
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_resolve_d8059bc113e215bf: function(arg0) {
                const ret = Promise.resolve(arg0);
                return ret;
            },
            __wbg_result_49cb3896934c8ef5: function() { return handleError(function (arg0) {
                const ret = arg0.result;
                return ret;
            }, arguments); },
            __wbg_send_da543a379e952bc6: function() { return handleError(function (arg0, arg1, arg2) {
                arg0.send(getArrayU8FromWasm0(arg1, arg2));
            }, arguments); },
            __wbg_setTimeout_ef24d2fc3ad97385: function() { return handleError(function (arg0, arg1) {
                const ret = setTimeout(arg0, arg1);
                return ret;
            }, arguments); },
            __wbg_set_0b4302959e9491f2: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4) {
                arg0.set(getStringFromWasm0(arg1, arg2), getStringFromWasm0(arg3, arg4));
            }, arguments); },
            __wbg_set_295bad3b5ead4e99: function(arg0, arg1, arg2) {
                arg0.set(getArrayU8FromWasm0(arg1, arg2));
            },
            __wbg_set_4702dfa37c77f492: function(arg0, arg1, arg2) {
                arg0[arg1 >>> 0] = arg2;
            },
            __wbg_set_6be42768c690e380: function(arg0, arg1, arg2) {
                arg0[arg1] = arg2;
            },
            __wbg_set_991082a7a49971cf: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = Reflect.set(arg0, arg1, arg2);
                return ret;
            }, arguments); },
            __wbg_set_auto_increment_1c19f677337bac29: function(arg0, arg1) {
                arg0.autoIncrement = arg1 !== 0;
            },
            __wbg_set_binaryType_0675f0e51c055ca8: function(arg0, arg1) {
                arg0.binaryType = __wbindgen_enum_BinaryType[arg1];
            },
            __wbg_set_body_e2cf9537a2f3e0be: function(arg0, arg1) {
                arg0.body = arg1;
            },
            __wbg_set_headers_22d4b01224273a83: function(arg0, arg1) {
                arg0.headers = arg1;
            },
            __wbg_set_key_path_7116eb54ff61da09: function(arg0, arg1) {
                arg0.keyPath = arg1;
            },
            __wbg_set_method_4a4ab3faba8a018c: function(arg0, arg1, arg2) {
                arg0.method = getStringFromWasm0(arg1, arg2);
            },
            __wbg_set_multi_entry_90808fb059650ead: function(arg0, arg1) {
                arg0.multiEntry = arg1 !== 0;
            },
            __wbg_set_name_49e1113b5a4e83ac: function(arg0, arg1, arg2) {
                arg0.name = getStringFromWasm0(arg1, arg2);
            },
            __wbg_set_onabort_befc75732a22eddc: function(arg0, arg1) {
                arg0.onabort = arg1;
            },
            __wbg_set_onclose_f791ef701be808a0: function(arg0, arg1) {
                arg0.onclose = arg1;
            },
            __wbg_set_oncomplete_929e8138b33e5033: function(arg0, arg1) {
                arg0.oncomplete = arg1;
            },
            __wbg_set_onerror_bf1e0e495c922c2f: function(arg0, arg1) {
                arg0.onerror = arg1;
            },
            __wbg_set_onerror_e23002e9224d353b: function(arg0, arg1) {
                arg0.onerror = arg1;
            },
            __wbg_set_onerror_f583e5ca2d87d320: function(arg0, arg1) {
                arg0.onerror = arg1;
            },
            __wbg_set_onmessage_d2fe701a9ce80846: function(arg0, arg1) {
                arg0.onmessage = arg1;
            },
            __wbg_set_onopen_0556381d0db30cbb: function(arg0, arg1) {
                arg0.onopen = arg1;
            },
            __wbg_set_onsuccess_45002542db0b8995: function(arg0, arg1) {
                arg0.onsuccess = arg1;
            },
            __wbg_set_onupgradeneeded_05b73da51749c9cd: function(arg0, arg1) {
                arg0.onupgradeneeded = arg1;
            },
            __wbg_set_onversionchange_77f1411c62a61a63: function(arg0, arg1) {
                arg0.onversionchange = arg1;
            },
            __wbg_set_signal_cd4528432ab8fe0b: function(arg0, arg1) {
                arg0.signal = arg1;
            },
            __wbg_set_unique_4db7742b21f4ad9d: function(arg0, arg1) {
                arg0.unique = arg1 !== 0;
            },
            __wbg_signal_6740ecf9bc372e29: function(arg0) {
                const ret = arg0.signal;
                return ret;
            },
            __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
                const ret = arg1.stack;
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_static_accessor_GLOBAL_8dfb7f5e26ebe523: function() {
                const ret = typeof global === 'undefined' ? null : global;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_static_accessor_GLOBAL_THIS_941154efc8395cdd: function() {
                const ret = typeof globalThis === 'undefined' ? null : globalThis;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_static_accessor_SELF_58dac9af822f561f: function() {
                const ret = typeof self === 'undefined' ? null : self;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_static_accessor_WINDOW_ee64f0b3d8354c0b: function() {
                const ret = typeof window === 'undefined' ? null : window;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_statusText_d47258d1f4a842f0: function(arg0, arg1) {
                const ret = arg1.statusText;
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_status_1ae443dc56281de7: function(arg0) {
                const ret = arg0.status;
                return ret;
            },
            __wbg_target_c2d80ee4d3287cbb: function(arg0) {
                const ret = arg0.target;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_then_0150352e4ad20344: function(arg0, arg1, arg2) {
                const ret = arg0.then(arg1, arg2);
                return ret;
            },
            __wbg_then_5160486c67ddb98a: function(arg0, arg1) {
                const ret = arg0.then(arg1);
                return ret;
            },
            __wbg_transaction_e87124a1a582309c: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.transaction(arg1, __wbindgen_enum_IdbTransactionMode[arg2]);
                return ret;
            }, arguments); },
            __wbg_transaction_fe7d473c90a75199: function(arg0) {
                const ret = arg0.transaction;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_unique_77bcc46d1c292903: function(arg0) {
                const ret = arg0.unique;
                return ret;
            },
            __wbg_url_900bb61156c69f05: function(arg0, arg1) {
                const ret = arg1.url;
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_value_d5b248ce8419bd1b: function(arg0) {
                const ret = arg0.value;
                return ret;
            },
            __wbg_warn_443d7dc3883e06f8: function(arg0, arg1, arg2, arg3) {
                console.warn(arg0, arg1, arg2, arg3);
            },
            __wbg_wasClean_2f24be63b9a84dc0: function(arg0) {
                const ret = arg0.wasClean;
                return ret;
            },
            __wbindgen_cast_0000000000000001: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [Externref], shim_idx: 53, ret: Result(Unit), inner_ret: Some(Result(Unit)) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h517d73105cbbbdea);
                return ret;
            },
            __wbindgen_cast_0000000000000002: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [Externref], shim_idx: 573, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h28fa93475e3b7691);
                return ret;
            },
            __wbindgen_cast_0000000000000003: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("CloseEvent")], shim_idx: 573, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h28fa93475e3b7691_2);
                return ret;
            },
            __wbindgen_cast_0000000000000004: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("Event")], shim_idx: 106, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__hbbf15603d2d57ea6);
                return ret;
            },
            __wbindgen_cast_0000000000000005: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("ExtendableMessageEvent")], shim_idx: 573, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h28fa93475e3b7691_4);
                return ret;
            },
            __wbindgen_cast_0000000000000006: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("IDBVersionChangeEvent")], shim_idx: 202, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h3db1ec1f2e22830d);
                return ret;
            },
            __wbindgen_cast_0000000000000007: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [NamedExternref("MessageEvent")], shim_idx: 573, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h28fa93475e3b7691_6);
                return ret;
            },
            __wbindgen_cast_0000000000000008: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [], shim_idx: 108, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__h3e06531f0c489366);
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
            __wbindgen_cast_000000000000000e: function(arg0, arg1) {
                var v0 = getArrayU8FromWasm0(arg0, arg1).slice();
                wasm.__wbindgen_free(arg0, arg1 * 1, 1);
                // Cast intrinsic for `Vector(U8) -> Externref`.
                const ret = v0;
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
            "./actr_runtime_sw_bg.js": import0,
        };
    }

    function wasm_bindgen__convert__closures_____invoke__h3e06531f0c489366(arg0, arg1) {
        wasm.wasm_bindgen__convert__closures_____invoke__h3e06531f0c489366(arg0, arg1);
    }

    function wasm_bindgen__convert__closures_____invoke__h28fa93475e3b7691(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h28fa93475e3b7691(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__h28fa93475e3b7691_2(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h28fa93475e3b7691_2(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__hbbf15603d2d57ea6(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__hbbf15603d2d57ea6(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__h28fa93475e3b7691_4(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h28fa93475e3b7691_4(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__h3db1ec1f2e22830d(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h3db1ec1f2e22830d(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__h28fa93475e3b7691_6(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h28fa93475e3b7691_6(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__h517d73105cbbbdea(arg0, arg1, arg2) {
        const ret = wasm.wasm_bindgen__convert__closures_____invoke__h517d73105cbbbdea(arg0, arg1, arg2);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }

    function wasm_bindgen__convert__closures_____invoke__h3ed608c0d4f5ac52(arg0, arg1, arg2, arg3) {
        wasm.wasm_bindgen__convert__closures_____invoke__h3ed608c0d4f5ac52(arg0, arg1, arg2, arg3);
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
        : new FinalizationRegistry(state => wasm.__wbindgen_destroy_closure(state.a, state.b));

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

    function makeMutClosure(arg0, arg1, f) {
        const state = { a: arg0, b: arg1, cnt: 1 };
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
                wasm.__wbindgen_destroy_closure(state.a, state.b);
                state.a = 0;
                CLOSURE_DTORS.unregister(state);
            }
        };
        CLOSURE_DTORS.register(real, state, state);
        return real;
    }

    function passArray8ToWasm0(arg, malloc) {
        const ptr = malloc(arg.length * 1, 1) >>> 0;
        getUint8ArrayMemory0().set(arg, ptr / 1);
        WASM_VECTOR_LEN = arg.length;
        return ptr;
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
