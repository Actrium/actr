import Actr
import Foundation
import SwiftProtobuf
import SwiftUI

// ACTR: mutable scaffold
// User-owned business scaffold. Keep EchoApp/Generated immutable and customize this file.

@MainActor
final class ActrService: ObservableObject {
    @Published var status = "Starting ACTR node..."
    @Published var errorMessage: String?

    private var actrNode: ActrNode?
    private var actorRef: ActrRef?
    private var linkedWorkload: DynamicWorkload?
    private var hasAutoSent = false
    private var isStarting = false

    var isReady: Bool {
        actorRef != nil
    }

    func startIfNeeded() async {
        guard actorRef == nil, !isStarting else { return }
        isStarting = true
        defer { isStarting = false }

        do {
            let configURL = try materializeRuntimeConfig()
            let echoServiceTarget = try readEchoServiceTarget()
            let actorType = ActrType(manufacturer: "acme", name: "EchoApp", version: "0.1.0")

            let workload = DynamicWorkload(
                lifecycle: LocalEchoServiceLifecycleAdapter(targetType: echoServiceTarget),
                signaling: nil,
                websocket: nil,
                webrtc: nil,
                credential: nil,
                mailbox: nil
            )
            linkedWorkload = workload
            let node = try await ActrNode.linked(config: configURL, type: actorType, workload: workload)

            actorRef = try await node.start()
            actrNode = node
            status = "Ready: \(actorType.toStringRepr())"
        } catch {
            status = "ACTR startup failed: \(error)"
            errorMessage = String(describing: error)
        }
    }

    func stop() async {
        guard let actorRef else { return }
        await actorRef.stop()
        self.actorRef = nil
        actrNode = nil
    }

    func sendEcho(_ input: String) async throws -> String {
        guard let actorRef else {
            throw EchoAppError.actorUnavailable
        }

        var request = Echoapp_LocalEchoRequest()
        request.message = input
        let response: Echoapp_LocalEchoResponse = try await actorRef.call(request)
        autoMarkSent()
        return response.reply
    }

    func shouldAutoSend() -> Bool {
        ProcessInfo.processInfo.environment["ACTR_ECHOAPP_AUTO_SEND"] == "1" && !hasAutoSent
    }

    private func autoMarkSent() {
        hasAutoSent = true
    }

    private func materializeRuntimeConfig() throws -> URL {
        guard let templateURL = Bundle.main.url(forResource: "actr", withExtension: "toml") else {
            throw EchoAppError.missingConfigTemplate
        }

        let fileManager = FileManager.default
        let supportURL = try fileManager.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
        let appURL = supportURL.appendingPathComponent("EchoApp", isDirectory: true)
        let dataURL = appURL.appendingPathComponent("hyper", isDirectory: true)
        try fileManager.createDirectory(at: dataURL, withIntermediateDirectories: true)

        var config = try String(contentsOf: templateURL, encoding: .utf8)
        config += """

        [hyper]
        data_dir = "\(dataURL.path)"

        [hyper.trust]
        kind = "dev_only"
        """

        let configURL = appURL.appendingPathComponent("actr.toml")
        try config.write(to: configURL, atomically: true, encoding: .utf8)
        return configURL
    }

    /// 从 bundled manifest.toml 中解析 echo 依赖的 ActrType（通过 actr FFI）
    private func readEchoServiceTarget() throws -> String {
        guard let url = Bundle.main.url(forResource: "manifest", withExtension: "toml") else {
            throw EchoAppError.missingManifest
        }
        let actrType = try ActrNode.resolveManifestDependency(
            manifestPath: url.path,
            dependencyAlias: "echo"
        )
        return actrType.toStringRepr()
    }
}

private enum EchoAppError: Error {
    case missingConfigTemplate
    case missingManifest
    case actorUnavailable
}

/// Safe: no mutable state. All fields are value types or immutable references.
private final class LocalEchoServiceHandlerImpl: LocalEchoServiceHandler, @unchecked Sendable {
    private let remoteEchoRoute = "echo.EchoService.Echo"
    private let targetType: String

    init(targetType: String) {
        self.targetType = targetType
    }

    func send(
        req localRequest: Echoapp_LocalEchoRequest,
        ctx: Context
    ) async throws -> Echoapp_LocalEchoResponse {
        let forwardedMessage = "swift-local:\(localRequest.message)"

        var remoteRequest = Echo_EchoRequest()
        remoteRequest.message = forwardedMessage
        let targetType = try ActrType.fromStringRepr(self.targetType)
        let targetId = try await ctx.discover(targetType: targetType)
        let remotePayload = try await ctx.call(
            target: targetId,
            routeKey: remoteEchoRoute,
            payload: remoteRequest.serializedData()
        )

        let remoteResponse = try Echo_EchoResponse(serializedBytes: remotePayload)
        var localResponse = Echoapp_LocalEchoResponse()
        localResponse.reply = remoteResponse.reply
        localResponse.forwardedMessage = forwardedMessage
        return localResponse
    }
}

/// Safe: no mutable state. `workload` is a let-bound actor.
private final class LocalEchoServiceLifecycleAdapter: Workload, @unchecked Sendable {
    private let workload: LocalEchoServiceWorkload<LocalEchoServiceHandlerImpl>

    init(targetType: String) {
        self.workload = LocalEchoServiceWorkload(handler: LocalEchoServiceHandlerImpl(targetType: targetType))
    }

    func onStart(ctx: ContextBridge) async throws {}

    func onReady(ctx: ContextBridge) async throws {}

    func onStop(ctx: ContextBridge) async throws {}

    func onError(ctx: ContextBridge, event: ErrorEventBridge) async throws {
        print("LocalEchoServiceLifecycleAdapter error: \(event)")
    }

    func dispatch(ctx: ContextBridge, envelope: RpcEnvelopeBridge) async throws -> Data {
        return try await workload.__dispatch(ctx: ctx, envelope: envelope)
    }
}
