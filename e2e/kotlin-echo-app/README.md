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

**Validated on CI (green):** the `kotlin-echo-app-e2e` job passes end-to-end on
ubuntu x86_64 + KVM + `reactivecircus/android-emulator-runner` — native `.so` + AAR build,
actrix + EchoService bootstrap, Kotlin scaffold + codegen, APK build/install, emulator boot,
and the `am instrument` echo round-trip all succeed.

Local macOS (Apple Silicon arm64 emulator) hits a WebRTC relay instability that does **not**
reproduce on CI: the TURN relayed data channel opens then closes before the RPC reply gets
through, and retries time out. So local macOS is not a reliable reproducer for this suite;
CI is the source of truth.

Two config notes (both handled by `run.sh`, flagged as repo-level debt):
- The shared actrix template's `enable = 25` = SIGNALING|AIS|SIGNER — the STUN(2)/TURN(4) bits
  are unset, so actrix ships with STUN/TURN disabled. This suite needs TURN (the emulator can't
  do direct ICE to the host), so `run.sh` bumps it to `enable = 31`. Cleaner: parameterize the
  shared template instead of the in-place `perl` rewrite (review finding #1, follow-up).
- actrix binds ICE on `0.0.0.0` and advertises the host LAN IP so the emulator can reach
  STUN/TURN (the emulator can't use 127.0.0.1).
