# Kotlin EchoApp E2E

Kotlin/Android nightly e2e, the mirror of [`swift-echo-app`](../swift-echo-app). A Rust
`EchoService` is scaffolded, built, published to a local actrix and run; a Kotlin Android
client (linked mode) calls `echo.EchoService.Echo` on it; the instrumentation test asserts the
reply equals `Echo: <message>`.

## Architecture

```
 Android emulator (x86_64 on CI / arm64-v8a on Apple Silicon)
   └─ KotlinEchoApp (linked ActrNode, scaffolded via `actr init -l kotlin --template echo`)
        │  reads actr.toml (10.0.2.2 → host) + manifest.toml
        │  UnifiedWorkload discovers the remote EchoService
        └─ echo.EchoService.Echo  ──▶  actrix (host, 127.0.0.1)  ──▶  Rust EchoService (`actr run`)
```

Verification is `./gradlew connectedDebugAndroidTest` (JUnit), not logcat greping — the
idiomatic Android pattern.

## Shared helpers

All actrix/server-lifecycle + Kotlin-app-scaffolding logic is factored into
[`../kotlin-lib/`](../kotlin-lib) and reused by the (planned) `kotlin-datastream-app` and
`kotlin-ts-workload` suites.

## How it differs from the Swift suite

- **Native libs are gitignored** — every run rebuilds the `.so` via
  `bindings/kotlin/build-android.sh` and publishes the AAR to `mavenLocal()`.
- **Linked mode, package-backed demo test notwithstanding** — the fixture instrumentation test
  (`cli/fixtures/kotlin/echo/EchoIntegrationTest.kt`) does not compile against the current DSL
  (`createActrNode` now requires a package path). This suite writes its own linked-mode test
  modeled on the demo app's `ClientActivity` (`ActrNode.linked` / `linkedWithMonitoring`).
- The Android emulator reaches the host via `10.0.2.2`; the bundled `actr.toml` uses that,
  while the host-side `manifest.toml` (consumed by `actr deps install`/`gen`) uses `127.0.0.1`.

## Running

CI: the `kotlin-echo-app-e2e` job in `.github/workflows/ci-e2e.yml` (nightly cron +
`workflow_dispatch`). It boots the emulator via `reactivecircus/android-emulator-runner`, then
runs `run.sh`.

Locally (Apple Silicon): build the native lib for the emulator arch, publish the AAR, boot an
arm64 emulator, then:

```bash
ACTR_ANDROID_TARGETS=aarch64-linux-android bash bindings/kotlin/build-android.sh
(cd bindings/kotlin && ./gradlew :actr-kotlin:publishToMavenLocal)
bash e2e/kotlin-echo-app/run.sh
```

A green run prints `actrix is healthy` → `EchoService readiness check complete` →
`BUILD SUCCESSFUL` from `connectedDebugAndroidTest`.

## Status / approach

The client-side flow follows the 0.4.x application pattern (`actr-kt-migration.md`):
`manifest.toml` (only `[package]` + `[dependencies]`) + `actr.toml` (connection + ACL) + a
minimal placeholder `manifest.lock.toml`, with the remote `echo.proto` placed locally and
`actr gen -l kotlin` consuming it via the `protos/` input. **No `actr deps install` / registry
round-trip is needed** for codegen.

Validated locally end-to-end through RPC dispatch: actrix bootstrap, MFR keychain, Rust
EchoService build + registry publish, server host start + signaling registration, Kotlin client
scaffold, local-proto + placeholder lock, `actr gen -l kotlin`, app + androidTest APK build +
install, instrumentation test run — the linked client starts, discovers
`actrium:EchoService:1.0.0`, and dispatches the `echo.EchoService.Echo` RPC.

The remaining failure is the **WebRTC relayed connection between the Android emulator and the
host**. Key findings (the earlier "`attribute not found` = TURN-protocol bug" diagnosis was
wrong — that error was only because STUN/TURN wasn't running):

- The shared actrix config had `enable = 25` = SIGNALING|AIS|SIGNER — **the STUN(2) and
  TURN(4) bits were unset**, so actrix logged `ICE服务(STUN/TURN)已禁用` and never started the
  STUN/TURN server. The e2e sets `enable = 31` to turn them on.
- With STUN/TURN running (and the sibling actrix that advertises the public IP), TURN ALLOCATE
  **succeeds**: both client and server receive valid HMAC time-limited TURN credentials
  (`username="<expiry>:<actor_id>"`) from registration, and the client gathers a relay
  candidate (`relay 10.30.3.206:NNNNN`).
- The relayed ICE connection still times out (`Connection attempt N/4 failed: timed out`). Both
  peers have credentials + the client has a relay candidate, but the relay ↔ relay bridging on
  actrix's TURN doesn't complete within the timeout.

Two repo-level items: (1) the actr repo's vendored `actrix/` is **stale** vs the sibling actrix
with the `advertise public IP` fix — CI must build the sibling (`ACTRIX_SOURCE_DIR`) or sync the
vendored copy; (2) the shared `enable = 25` disables STUN/TURN, so any e2e needing TURN (this
one) must bump it. The remaining relay-bridging timeout is the last WebRTC piece; everything
else (build, install, gen, discovery, RPC dispatch, TURN allocate, relay-candidate gather) is
validated.
