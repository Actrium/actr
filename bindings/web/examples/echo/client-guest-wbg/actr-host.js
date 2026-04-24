// SPDX-License-Identifier: Apache-2.0
//
// Stub JS shim for the 8 `actrHost*` functions imported by
// `actr-web-abi::guest`. Phase 4a only needs the stubs to *exist* so
// that consumers of the wasm-pack `pkg/` can wire glue + wasm without
// TypeError on instantiation; actual bridging to `sw-host` is Phase 4c.
//
// Load order (in a Service Worker or DOM page):
//   importScripts('./actr-host.js');                    // install globals
//   importScripts('./pkg/echo_server_guest_wbg.js');    // consumes them
//
// All functions return Promises to match `actr-web-abi::guest`, which
// uniformly awaits `JsFuture::from(__actr_host_*)`.

(function (scope) {
  'use strict';

  const note = (fn, args) =>
    console.debug('[actr-host-stub]', fn, args);

  // `actr-id` shaped exactly like the generated `ActrId` record —
  // kebab-cased field names are load-bearing (see types.rs
  // `#[serde(rename = "serial-number")]`).
  const stubActrId = () => ({
    realm: { 'realm-id': 0 },
    'serial-number': 1n,
    type: {
      manufacturer: 'stub',
      name: 'StubActr',
      version: '0.0.0',
    },
  });

  scope.actrHostCall = function (target, routeKey, payload) {
    note('actrHostCall', { target, routeKey, byteLen: payload && payload.length });
    // `Result<Vec<u8>, ActrError>` — success shape is `{ Ok: Uint8Array }`.
    return Promise.resolve({ Ok: new Uint8Array() });
  };

  scope.actrHostCallRaw = function (target, routeKey, payload) {
    note('actrHostCallRaw', { target, routeKey, byteLen: payload && payload.length });
    return Promise.resolve({ Ok: new Uint8Array() });
  };

  scope.actrHostDiscover = function (targetType) {
    note('actrHostDiscover', targetType);
    return Promise.resolve({ Ok: stubActrId() });
  };

  scope.actrHostGetCallerId = function () {
    note('actrHostGetCallerId');
    // `Option<ActrId>` — `null` maps to `None`.
    return Promise.resolve(null);
  };

  scope.actrHostGetRequestId = function () {
    note('actrHostGetRequestId');
    return Promise.resolve('stub-request-id');
  };

  scope.actrHostGetSelfId = function () {
    note('actrHostGetSelfId');
    return Promise.resolve(stubActrId());
  };

  scope.actrHostLogMessage = function (level, message) {
    note('actrHostLogMessage', { level, message });
    return Promise.resolve();
  };

  scope.actrHostTell = function (target, routeKey, payload) {
    note('actrHostTell', { target, routeKey, byteLen: payload && payload.length });
    // `Result<(), ActrError>` — success shape is `{ Ok: null }`.
    return Promise.resolve({ Ok: null });
  };
})(typeof globalThis !== 'undefined' ? globalThis : self);
