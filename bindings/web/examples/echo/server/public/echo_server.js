let wasm_bindgen = (function(exports) {
    let script_src;
    if (typeof document !== 'undefined' && document.currentScript !== null) {
        script_src = new URL(document.currentScript.src, location.href).toString();
    }

    /**
     * 处理来自 DOM 的 RPC 控制请求
     *
     * 消息流（Unified Dispatcher 模式）：
     * - 有 SERVICE_HANDLER: DOM → handler(route_key, payload, ctx) → response
     *   - local route: handler 本地处理，可通过 ctx.call_raw() 调远程
     *   - remote route: handler 通过 ctx.call_raw() 转发到远程 Actor
     * - 无 SERVICE_HANDLER: DOM → InprocOutGate → OutGate → WebRTC（旧路径，向后兼容）
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
     * WASM 初始化入口
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
     * 注册来自 DOM 的专用 DataChannel MessagePort
     *
     * DOM 在 DataChannel 建立后创建 MessageChannel 桥接：
     * 1. DOM: `port1 ↔ DataChannel` (双向转发)
     * 2. DOM: 将 `port2` 通过 Transferable 转移给 SW
     * 3. SW: 此函数接收 `port2`，创建 `WebRtcConnection`，注入到 `WirePool`
     *
     * 注入后 DestTransport 的 send 循环通过 ReadyWatcher 被唤醒，
     * 后续出站数据直接经 `DataLane::PostMessage(port)` 零拷贝发送。
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
     * 注册 EchoService handler
     *
     * handler 分发 RPC 请求到 EchoService 的具体方法，
     * 并将 RuntimeContext 透传给每个方法。
     */
    function register_echo_service() {
        wasm.register_echo_service();
    }
    exports.register_echo_service = register_echo_service;

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
            __wbg_Error_8c4e43fe74559d73: function(arg0, arg1) {
                const ret = Error(getStringFromWasm0(arg0, arg1));
                return ret;
            },
            __wbg_Number_04624de7d0e8332d: function(arg0) {
                const ret = Number(arg0);
                return ret;
            },
            __wbg_String_8f0eb39a4a4c2f66: function(arg0, arg1) {
                const ret = String(arg1);
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg___wbindgen_bigint_get_as_i64_8fcf4ce7f1ca72a2: function(arg0, arg1) {
                const v = arg1;
                const ret = typeof(v) === 'bigint' ? v : undefined;
                getDataViewMemory0().setBigInt64(arg0 + 8 * 1, isLikeNone(ret) ? BigInt(0) : ret, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
            },
            __wbg___wbindgen_boolean_get_bbbb1c18aa2f5e25: function(arg0) {
                const v = arg0;
                const ret = typeof(v) === 'boolean' ? v : undefined;
                return isLikeNone(ret) ? 0xFFFFFF : ret ? 1 : 0;
            },
            __wbg___wbindgen_debug_string_0bc8482c6e3508ae: function(arg0, arg1) {
                const ret = debugString(arg1);
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg___wbindgen_in_47fa6863be6f2f25: function(arg0, arg1) {
                const ret = arg0 in arg1;
                return ret;
            },
            __wbg___wbindgen_is_bigint_31b12575b56f32fc: function(arg0) {
                const ret = typeof(arg0) === 'bigint';
                return ret;
            },
            __wbg___wbindgen_is_function_0095a73b8b156f76: function(arg0) {
                const ret = typeof(arg0) === 'function';
                return ret;
            },
            __wbg___wbindgen_is_null_ac34f5003991759a: function(arg0) {
                const ret = arg0 === null;
                return ret;
            },
            __wbg___wbindgen_is_object_5ae8e5880f2c1fbd: function(arg0) {
                const val = arg0;
                const ret = typeof(val) === 'object' && val !== null;
                return ret;
            },
            __wbg___wbindgen_is_string_cd444516edc5b180: function(arg0) {
                const ret = typeof(arg0) === 'string';
                return ret;
            },
            __wbg___wbindgen_is_undefined_9e4d92534c42d778: function(arg0) {
                const ret = arg0 === undefined;
                return ret;
            },
            __wbg___wbindgen_jsval_eq_11888390b0186270: function(arg0, arg1) {
                const ret = arg0 === arg1;
                return ret;
            },
            __wbg___wbindgen_jsval_loose_eq_9dd77d8cd6671811: function(arg0, arg1) {
                const ret = arg0 == arg1;
                return ret;
            },
            __wbg___wbindgen_number_get_8ff4255516ccad3e: function(arg0, arg1) {
                const obj = arg1;
                const ret = typeof(obj) === 'number' ? obj : undefined;
                getDataViewMemory0().setFloat64(arg0 + 8 * 1, isLikeNone(ret) ? 0 : ret, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
            },
            __wbg___wbindgen_string_get_72fb696202c56729: function(arg0, arg1) {
                const obj = arg1;
                const ret = typeof(obj) === 'string' ? obj : undefined;
                var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                var len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg___wbindgen_throw_be289d5034ed271b: function(arg0, arg1) {
                throw new Error(getStringFromWasm0(arg0, arg1));
            },
            __wbg__wbg_cb_unref_d9b87ff7982e3b21: function(arg0) {
                arg0._wbg_cb_unref();
            },
            __wbg_addEventListener_3acb0aad4483804c: function() { return handleError(function (arg0, arg1, arg2, arg3) {
                arg0.addEventListener(getStringFromWasm0(arg1, arg2), arg3);
            }, arguments); },
            __wbg_add_1a3b49f26498f88c: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.add(arg1);
                return ret;
            }, arguments); },
            __wbg_add_7efeacc1aa78048a: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.add(arg1, arg2);
                return ret;
            }, arguments); },
            __wbg_call_389efe28435a9388: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.call(arg1);
                return ret;
            }, arguments); },
            __wbg_call_4708e0c13bdc8e95: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.call(arg1, arg2);
                return ret;
            }, arguments); },
            __wbg_clearTimeout_5a54f8841c30079a: function(arg0) {
                const ret = clearTimeout(arg0);
                return ret;
            },
            __wbg_clear_159551fa0f231a1d: function() { return handleError(function (arg0) {
                const ret = arg0.clear();
                return ret;
            }, arguments); },
            __wbg_close_1d08eaf57ed325c0: function() { return handleError(function (arg0) {
                arg0.close();
            }, arguments); },
            __wbg_close_53683f4809368fc7: function(arg0) {
                arg0.close();
            },
            __wbg_code_a552f1e91eda69b7: function(arg0) {
                const ret = arg0.code;
                return ret;
            },
            __wbg_createIndex_9a2be04a017f6a17: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4) {
                const ret = arg0.createIndex(getStringFromWasm0(arg1, arg2), arg3, arg4);
                return ret;
            }, arguments); },
            __wbg_createObjectStore_f75f59d55a549868: function() { return handleError(function (arg0, arg1, arg2, arg3) {
                const ret = arg0.createObjectStore(getStringFromWasm0(arg1, arg2), arg3);
                return ret;
            }, arguments); },
            __wbg_data_1a8eefa0288b4e4a: function(arg0) {
                const ret = arg0.data;
                return ret;
            },
            __wbg_data_5330da50312d0bc1: function(arg0) {
                const ret = arg0.data;
                return ret;
            },
            __wbg_debug_46a93995fc6f8820: function(arg0, arg1, arg2, arg3) {
                console.debug(arg0, arg1, arg2, arg3);
            },
            __wbg_deleteIndex_0a7a8e99fe536b7c: function() { return handleError(function (arg0, arg1, arg2) {
                arg0.deleteIndex(getStringFromWasm0(arg1, arg2));
            }, arguments); },
            __wbg_deleteObjectStore_6f911570c372b5f6: function() { return handleError(function (arg0, arg1, arg2) {
                arg0.deleteObjectStore(getStringFromWasm0(arg1, arg2));
            }, arguments); },
            __wbg_delete_d6d7f750bd9ed2cd: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.delete(arg1);
                return ret;
            }, arguments); },
            __wbg_done_57b39ecd9addfe81: function(arg0) {
                const ret = arg0.done;
                return ret;
            },
            __wbg_entries_58c7934c745daac7: function(arg0) {
                const ret = Object.entries(arg0);
                return ret;
            },
            __wbg_error_6afb95c784775817: function() { return handleError(function (arg0) {
                const ret = arg0.error;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            }, arguments); },
            __wbg_error_7534b8e9a36f1ab4: function(arg0, arg1) {
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
            __wbg_error_794d0ffc9d00d5c3: function(arg0, arg1, arg2, arg3) {
                console.error(arg0, arg1, arg2, arg3);
            },
            __wbg_error_9a7fe3f932034cde: function(arg0) {
                console.error(arg0);
            },
            __wbg_error_bf9fa99d609a0ce7: function(arg0) {
                const ret = arg0.error;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_getAll_0a56b25474b504d0: function() { return handleError(function (arg0, arg1) {
                const ret = arg0.getAll(arg1);
                return ret;
            }, arguments); },
            __wbg_getAll_33c9f4f22da09509: function() { return handleError(function (arg0) {
                const ret = arg0.getAll();
                return ret;
            }, arguments); },
            __wbg_getAll_4a87f2b2a7e22ea5: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.getAll(arg1, arg2 >>> 0);
                return ret;
            }, arguments); },
            __wbg_getRandomValues_9c5c1b115e142bb8: function() { return handleError(function (arg0, arg1) {
                globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
            }, arguments); },
            __wbg_get_626204a85e34f823: function(arg0, arg1, arg2) {
                const ret = arg1[arg2 >>> 0];
                var ptr1 = isLikeNone(ret) ? 0 : passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                var len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_get_9b94d73e6221f75c: function(arg0, arg1) {
                const ret = arg0[arg1 >>> 0];
                return ret;
            },
            __wbg_get_b3ed3ad4be2bc8ac: function() { return handleError(function (arg0, arg1) {
                const ret = Reflect.get(arg0, arg1);
                return ret;
            }, arguments); },
            __wbg_get_with_ref_key_1dc361bd10053bfe: function(arg0, arg1) {
                const ret = arg0[arg1];
                return ret;
            },
            __wbg_indexNames_e2c333fa9895469f: function(arg0) {
                const ret = arg0.indexNames;
                return ret;
            },
            __wbg_index_f2a34128c0806ae8: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.index(getStringFromWasm0(arg1, arg2));
                return ret;
            }, arguments); },
            __wbg_info_9e602cf10c5c690b: function(arg0, arg1, arg2, arg3) {
                console.info(arg0, arg1, arg2, arg3);
            },
            __wbg_instanceof_ArrayBuffer_c367199e2fa2aa04: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof ArrayBuffer;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbDatabase_8d723b3ff4761c2d: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBDatabase;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbFactory_39d4fb6425cae0a6: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBFactory;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbOpenDbRequest_e476921a744b955b: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBOpenDBRequest;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbRequest_6388508cc77f8da0: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBRequest;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_IdbTransaction_ec5dc92e602db81d: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof IDBTransaction;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_Map_53af74335dec57f4: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof Map;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_Object_1c6af87502b733ed: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof Object;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_ServiceWorkerGlobalScope_0d22bcd128146294: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof ServiceWorkerGlobalScope;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_instanceof_Uint8Array_9b9075935c74707c: function(arg0) {
                let result;
                try {
                    result = arg0 instanceof Uint8Array;
                } catch (_) {
                    result = false;
                }
                const ret = result;
                return ret;
            },
            __wbg_isArray_d314bb98fcf08331: function(arg0) {
                const ret = Array.isArray(arg0);
                return ret;
            },
            __wbg_isSafeInteger_bfbc7332a9768d2a: function(arg0) {
                const ret = Number.isSafeInteger(arg0);
                return ret;
            },
            __wbg_iterator_6ff6560ca1568e55: function() {
                const ret = Symbol.iterator;
                return ret;
            },
            __wbg_keyPath_ac4813ee38214f4e: function() { return handleError(function (arg0) {
                const ret = arg0.keyPath;
                return ret;
            }, arguments); },
            __wbg_length_32ed9a279acd054c: function(arg0) {
                const ret = arg0.length;
                return ret;
            },
            __wbg_length_35a7bace40f36eac: function(arg0) {
                const ret = arg0.length;
                return ret;
            },
            __wbg_length_4c6eb4059a3635c9: function(arg0) {
                const ret = arg0.length;
                return ret;
            },
            __wbg_log_24aba2a6d8990b35: function(arg0, arg1, arg2, arg3) {
                console.log(arg0, arg1, arg2, arg3);
            },
            __wbg_log_6b5ca2e6124b2808: function(arg0) {
                console.log(arg0);
            },
            __wbg_multiEntry_297e525177ee3dd7: function(arg0) {
                const ret = arg0.multiEntry;
                return ret;
            },
            __wbg_new_057993d5b5e07835: function() { return handleError(function (arg0, arg1) {
                const ret = new WebSocket(getStringFromWasm0(arg0, arg1));
                return ret;
            }, arguments); },
            __wbg_new_361308b2356cecd0: function() {
                const ret = new Object();
                return ret;
            },
            __wbg_new_3eb36ae241fe6f44: function() {
                const ret = new Array();
                return ret;
            },
            __wbg_new_8a6f238a6ece86ea: function() {
                const ret = new Error();
                return ret;
            },
            __wbg_new_b5d9e2fb389fef91: function(arg0, arg1) {
                try {
                    var state0 = {a: arg0, b: arg1};
                    var cb0 = (arg0, arg1) => {
                        const a = state0.a;
                        state0.a = 0;
                        try {
                            return wasm_bindgen__convert__closures_____invoke__h4e7c82544a309da3(a, state0.b, arg0, arg1);
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
            __wbg_new_dd2b680c8bf6ae29: function(arg0) {
                const ret = new Uint8Array(arg0);
                return ret;
            },
            __wbg_new_from_slice_a3d2629dc1826784: function(arg0, arg1) {
                const ret = new Uint8Array(getArrayU8FromWasm0(arg0, arg1));
                return ret;
            },
            __wbg_new_no_args_1c7c842f08d00ebb: function(arg0, arg1) {
                const ret = new Function(getStringFromWasm0(arg0, arg1));
                return ret;
            },
            __wbg_next_3482f54c49e8af19: function() { return handleError(function (arg0) {
                const ret = arg0.next();
                return ret;
            }, arguments); },
            __wbg_next_418f80d8f5303233: function(arg0) {
                const ret = arg0.next;
                return ret;
            },
            __wbg_now_a3af9a2f4bbaa4d1: function() {
                const ret = Date.now();
                return ret;
            },
            __wbg_objectStoreNames_d2c5d2377420ad78: function(arg0) {
                const ret = arg0.objectStoreNames;
                return ret;
            },
            __wbg_objectStore_d56e603390dcc165: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.objectStore(getStringFromWasm0(arg1, arg2));
                return ret;
            }, arguments); },
            __wbg_open_1b21db8aeca0eea9: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.open(getStringFromWasm0(arg1, arg2));
                return ret;
            }, arguments); },
            __wbg_open_82db86fd5b087109: function() { return handleError(function (arg0, arg1, arg2, arg3) {
                const ret = arg0.open(getStringFromWasm0(arg1, arg2), arg3 >>> 0);
                return ret;
            }, arguments); },
            __wbg_postMessage_46eeeef39934b448: function() { return handleError(function (arg0, arg1) {
                arg0.postMessage(arg1);
            }, arguments); },
            __wbg_prototypesetcall_bdcdcc5842e4d77d: function(arg0, arg1, arg2) {
                Uint8Array.prototype.set.call(getArrayU8FromWasm0(arg0, arg1), arg2);
            },
            __wbg_push_8ffdcb2063340ba5: function(arg0, arg1) {
                const ret = arg0.push(arg1);
                return ret;
            },
            __wbg_queueMicrotask_0aa0a927f78f5d98: function(arg0) {
                const ret = arg0.queueMicrotask;
                return ret;
            },
            __wbg_queueMicrotask_5bb536982f78a56f: function(arg0) {
                queueMicrotask(arg0);
            },
            __wbg_readyState_1bb73ec7b8a54656: function(arg0) {
                const ret = arg0.readyState;
                return ret;
            },
            __wbg_reason_35fce8e55dd90f31: function(arg0, arg1) {
                const ret = arg1.reason;
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_resolve_002c4b7d9d8f6b64: function(arg0) {
                const ret = Promise.resolve(arg0);
                return ret;
            },
            __wbg_result_233b2d68aae87a05: function() { return handleError(function (arg0) {
                const ret = arg0.result;
                return ret;
            }, arguments); },
            __wbg_send_542f95dea2df7994: function() { return handleError(function (arg0, arg1, arg2) {
                arg0.send(getArrayU8FromWasm0(arg1, arg2));
            }, arguments); },
            __wbg_setTimeout_db2dbaeefb6f39c7: function() { return handleError(function (arg0, arg1) {
                const ret = setTimeout(arg0, arg1);
                return ret;
            }, arguments); },
            __wbg_set_3f1d0b984ed272ed: function(arg0, arg1, arg2) {
                arg0[arg1] = arg2;
            },
            __wbg_set_6cb8631f80447a67: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = Reflect.set(arg0, arg1, arg2);
                return ret;
            }, arguments); },
            __wbg_set_auto_increment_5ef604f4f193fa58: function(arg0, arg1) {
                arg0.autoIncrement = arg1 !== 0;
            },
            __wbg_set_binaryType_5bbf62e9f705dc1a: function(arg0, arg1) {
                arg0.binaryType = __wbindgen_enum_BinaryType[arg1];
            },
            __wbg_set_f43e577aea94465b: function(arg0, arg1, arg2) {
                arg0[arg1 >>> 0] = arg2;
            },
            __wbg_set_key_path_d4c32b4460a1f7d7: function(arg0, arg1) {
                arg0.keyPath = arg1;
            },
            __wbg_set_multi_entry_1e0edb6570bebc20: function(arg0, arg1) {
                arg0.multiEntry = arg1 !== 0;
            },
            __wbg_set_name_e71c1e088429e6f9: function(arg0, arg1, arg2) {
                arg0.name = getStringFromWasm0(arg1, arg2);
            },
            __wbg_set_onabort_5b85743a64489257: function(arg0, arg1) {
                arg0.onabort = arg1;
            },
            __wbg_set_onclose_d382f3e2c2b850eb: function(arg0, arg1) {
                arg0.onclose = arg1;
            },
            __wbg_set_oncomplete_76d4a772a6c8cab6: function(arg0, arg1) {
                arg0.oncomplete = arg1;
            },
            __wbg_set_onerror_377f18bf4569bf85: function(arg0, arg1) {
                arg0.onerror = arg1;
            },
            __wbg_set_onerror_d0db7c6491b9399d: function(arg0, arg1) {
                arg0.onerror = arg1;
            },
            __wbg_set_onerror_dc0e606b09e1792f: function(arg0, arg1) {
                arg0.onerror = arg1;
            },
            __wbg_set_onmessage_2114aa5f4f53051e: function(arg0, arg1) {
                arg0.onmessage = arg1;
            },
            __wbg_set_onopen_b7b52d519d6c0f11: function(arg0, arg1) {
                arg0.onopen = arg1;
            },
            __wbg_set_onsuccess_0edec1acb4124784: function(arg0, arg1) {
                arg0.onsuccess = arg1;
            },
            __wbg_set_onupgradeneeded_c887b74722b6ce77: function(arg0, arg1) {
                arg0.onupgradeneeded = arg1;
            },
            __wbg_set_onversionchange_34b86d0aaffbe107: function(arg0, arg1) {
                arg0.onversionchange = arg1;
            },
            __wbg_set_unique_9609afeaaff95e61: function(arg0, arg1) {
                arg0.unique = arg1 !== 0;
            },
            __wbg_stack_0ed75d68575b0f3c: function(arg0, arg1) {
                const ret = arg1.stack;
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_static_accessor_GLOBAL_12837167ad935116: function() {
                const ret = typeof global === 'undefined' ? null : global;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_static_accessor_GLOBAL_THIS_e628e89ab3b1c95f: function() {
                const ret = typeof globalThis === 'undefined' ? null : globalThis;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_static_accessor_SELF_a621d3dfbb60d0ce: function() {
                const ret = typeof self === 'undefined' ? null : self;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_static_accessor_WINDOW_f8727f0cf888e0bd: function() {
                const ret = typeof window === 'undefined' ? null : window;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_target_521be630ab05b11e: function(arg0) {
                const ret = arg0.target;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_then_b9e7b3b5f1a9e1b5: function(arg0, arg1) {
                const ret = arg0.then(arg1);
                return ret;
            },
            __wbg_transaction_5124caf7db668498: function(arg0) {
                const ret = arg0.transaction;
                return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
            },
            __wbg_transaction_c407989db8e62119: function() { return handleError(function (arg0, arg1, arg2) {
                const ret = arg0.transaction(arg1, __wbindgen_enum_IdbTransactionMode[arg2]);
                return ret;
            }, arguments); },
            __wbg_unique_42f9a655b8811f67: function(arg0) {
                const ret = arg0.unique;
                return ret;
            },
            __wbg_url_cb4d34db86c24df9: function(arg0, arg1) {
                const ret = arg1.url;
                const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
                const len1 = WASM_VECTOR_LEN;
                getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
                getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
            },
            __wbg_value_0546255b415e96c1: function(arg0) {
                const ret = arg0.value;
                return ret;
            },
            __wbg_warn_a40b971467b219c7: function(arg0, arg1, arg2, arg3) {
                console.warn(arg0, arg1, arg2, arg3);
            },
            __wbg_wasClean_a9c77a7100d8534f: function(arg0) {
                const ret = arg0.wasClean;
                return ret;
            },
            __wbindgen_cast_0000000000000001: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 173, function: Function { arguments: [NamedExternref("CloseEvent")], shim_idx: 174, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h0042e06db519aef1, wasm_bindgen__convert__closures_____invoke__h0a566f92f114a454);
                return ret;
            },
            __wbindgen_cast_0000000000000002: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 173, function: Function { arguments: [NamedExternref("ExtendableMessageEvent")], shim_idx: 174, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h0042e06db519aef1, wasm_bindgen__convert__closures_____invoke__h0a566f92f114a454);
                return ret;
            },
            __wbindgen_cast_0000000000000003: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 173, function: Function { arguments: [NamedExternref("IDBVersionChangeEvent")], shim_idx: 174, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h0042e06db519aef1, wasm_bindgen__convert__closures_____invoke__h0a566f92f114a454);
                return ret;
            },
            __wbindgen_cast_0000000000000004: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 173, function: Function { arguments: [NamedExternref("MessageEvent")], shim_idx: 174, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h0042e06db519aef1, wasm_bindgen__convert__closures_____invoke__h0a566f92f114a454);
                return ret;
            },
            __wbindgen_cast_0000000000000005: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 254, function: Function { arguments: [], shim_idx: 255, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h0698ac6d5e870c6f, wasm_bindgen__convert__closures_____invoke__hbb7884ff57f32b59);
                return ret;
            },
            __wbindgen_cast_0000000000000006: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 260, function: Function { arguments: [Externref], shim_idx: 261, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h2f88ba97fc44e418, wasm_bindgen__convert__closures_____invoke__hccb1dd53255a5b11);
                return ret;
            },
            __wbindgen_cast_0000000000000007: function(arg0, arg1) {
                // Cast intrinsic for `Closure(Closure { dtor_idx: 330, function: Function { arguments: [NamedExternref("Event")], shim_idx: 331, ret: Unit, inner_ret: Some(Unit) }, mutable: true }) -> Externref`.
                const ret = makeMutClosure(arg0, arg1, wasm.wasm_bindgen__closure__destroy__h9972ac9645377338, wasm_bindgen__convert__closures_____invoke__h8d7c31f68d603512);
                return ret;
            },
            __wbindgen_cast_0000000000000008: function(arg0) {
                // Cast intrinsic for `F64 -> Externref`.
                const ret = arg0;
                return ret;
            },
            __wbindgen_cast_0000000000000009: function(arg0) {
                // Cast intrinsic for `I64 -> Externref`.
                const ret = arg0;
                return ret;
            },
            __wbindgen_cast_000000000000000a: function(arg0, arg1) {
                // Cast intrinsic for `Ref(Slice(U8)) -> NamedExternref("Uint8Array")`.
                const ret = getArrayU8FromWasm0(arg0, arg1);
                return ret;
            },
            __wbindgen_cast_000000000000000b: function(arg0, arg1) {
                // Cast intrinsic for `Ref(String) -> Externref`.
                const ret = getStringFromWasm0(arg0, arg1);
                return ret;
            },
            __wbindgen_cast_000000000000000c: function(arg0) {
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
            "./echo_server_bg.js": import0,
        };
    }

    function wasm_bindgen__convert__closures_____invoke__hbb7884ff57f32b59(arg0, arg1) {
        wasm.wasm_bindgen__convert__closures_____invoke__hbb7884ff57f32b59(arg0, arg1);
    }

    function wasm_bindgen__convert__closures_____invoke__h0a566f92f114a454(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h0a566f92f114a454(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__hccb1dd53255a5b11(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__hccb1dd53255a5b11(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__h8d7c31f68d603512(arg0, arg1, arg2) {
        wasm.wasm_bindgen__convert__closures_____invoke__h8d7c31f68d603512(arg0, arg1, arg2);
    }

    function wasm_bindgen__convert__closures_____invoke__h4e7c82544a309da3(arg0, arg1, arg2, arg3) {
        wasm.wasm_bindgen__convert__closures_____invoke__h4e7c82544a309da3(arg0, arg1, arg2, arg3);
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
