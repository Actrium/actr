import Actr
import Foundation

private enum AudioCaptureConfig {
    static let trackId = "audio-0"
    static let sampleRate: UInt32 = 48_000
    static let channels: UInt8 = 1
    static let frameSize: UInt16 = 960
}

/// Workload that captures microphone audio and sends it via MediaTrack.
final class AudioCaptureWorkload: @unchecked Sendable {
    private var system: ActrSystem?
    private var actrRef: ActrRef?
    private var context: Context?
    private var targetId: ActrId?
    private let audioEngine = AudioEngine()
    private let mediaSender: MediaSender

    init() async throws {
        let encoder = try OpusEncoder(
            sampleRate: AudioCaptureConfig.sampleRate,
            channels: AudioCaptureConfig.channels,
            frameSize: AudioCaptureConfig.frameSize
        )
        self.mediaSender = MediaSender(encoder: encoder)

        guard let configURL = Bundle.module.url(forResource: "Actr", withExtension: "toml") else {
            throw ActrError.ConfigError(msg: "Missing bundled Actr.toml")
        }

        let system = try await ActrSystem.from(tomlConfig: configURL)
        self.system = system

        let workload = AudioCaptureWorkloadBridge(owner: self)
        let node = try system.spawn(workload: workload)
        let actrRef = try await node.start()
        self.actrRef = actrRef
    }

    func setContext(_ ctx: Context) {
        self.context = ctx
        Task {
            await mediaSender.setContext(ctx)
        }
    }

    func startCapture() async throws {
        guard let context, let actrRef else {
            throw ActrError.InternalError(msg: "Not connected")
        }

        guard await AudioEngine.requestMicrophoneAccess() else {
            throw AudioEngineError.microphoneAccessDenied
        }

        let targetType = ActrType(manufacturer: "acme", name: "AudioRecorder", version: nil)
        let targets = try await actrRef.discover(targetType: targetType, count: 1)
        guard let target = targets.first else {
            throw ActrError.InternalError(msg: "AudioRecorder not found")
        }

        self.targetId = target

        try await context.addMediaTrack(
            target: target,
            trackId: AudioCaptureConfig.trackId,
            codec: "OPUS",
            mediaType: "audio"
        )
        await mediaSender.startSession(context: context, target: target)

        do {
            try audioEngine.start { [weak self] frame in
                guard let self else { return }
                Task {
                    await self.mediaSender.send(frame: frame)
                }
            }
        } catch {
            await mediaSender.stopSession()
            self.targetId = nil
            try? await context.removeMediaTrack(
                target: target,
                trackId: AudioCaptureConfig.trackId
            )
            throw error
        }
    }

    func stopCapture() async throws {
        audioEngine.stop()
        await mediaSender.stopSession()

        if let context, let target = targetId {
            try await context.removeMediaTrack(
                target: target,
                trackId: AudioCaptureConfig.trackId
            )
        }

        targetId = nil
    }
}

private actor MediaSender {
    private let encoder: OpusEncoder
    private var context: Context?
    private var target: ActrId?
    private var nextTimestamp: UInt32 = 0

    init(encoder: OpusEncoder) {
        self.encoder = encoder
    }

    func setContext(_ context: Context) {
        self.context = context
    }

    func startSession(context: Context, target: ActrId) {
        self.context = context
        self.target = target
        nextTimestamp = 0
    }

    func stopSession() {
        target = nil
        nextTimestamp = 0
    }

    func send(frame: [Float]) async {
        guard let context, let target else { return }

        do {
            let packet = try encoder.encode(pcm: frame)
            let sample = MediaSample(
                data: packet,
                timestamp: nextTimestamp,
                codec: "OPUS",
                mediaType: .audio
            )
            nextTimestamp &+= UInt32(frame.count)

            try await context.sendMediaSample(
                target: target,
                trackId: AudioCaptureConfig.trackId,
                sample: sample
            )
        } catch {
            print("MediaSender error: \(error.localizedDescription)")
        }
    }
}

// MARK: - WorkloadBridge implementation

private final class AudioCaptureWorkloadBridge: Workload, @unchecked Sendable {
    private weak var owner: AudioCaptureWorkload?

    init(owner: AudioCaptureWorkload) {
        self.owner = owner
    }

    func onStart(ctx: Context) async throws {
        owner?.setContext(ctx)
    }

    func onStop(ctx: Context) async throws {
        // no-op
    }

    func dispatch(ctx: Context, envelope: RpcEnvelope) async throws -> Data {
        throw ActrError.InternalError(msg: "AudioCapture has no RPC handlers")
    }
}
