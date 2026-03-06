@preconcurrency import AVFoundation

enum AudioEngineError: LocalizedError {
    case microphoneAccessDenied
    case converterUnavailable
    case bufferAllocationFailed
    case conversionFailed(String)

    var errorDescription: String? {
        switch self {
        case .microphoneAccessDenied:
            return "Microphone access was denied."
        case .converterUnavailable:
            return "Failed to configure audio converter."
        case .bufferAllocationFailed:
            return "Failed to allocate audio buffer."
        case .conversionFailed(let message):
            return "Audio conversion failed: \(message)"
        }
    }
}

/// Captures microphone audio, converts it to 48kHz mono PCM float frames,
/// and emits fixed 20ms chunks ready for Opus encoding.
final class AudioEngine {
    static let sampleRate: Double = 48_000
    static let channels: AVAudioChannelCount = 1
    static let frameSize = 960

    private let engine = AVAudioEngine()
    private var converter: AVAudioConverter?
    private var onFrame: (([Float]) -> Void)?
    private var pendingSamples: [Float] = []

    static func requestMicrophoneAccess() async -> Bool {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            return true
        case .notDetermined:
            return await withCheckedContinuation { continuation in
                AVCaptureDevice.requestAccess(for: .audio) { granted in
                    continuation.resume(returning: granted)
                }
            }
        case .denied, .restricted:
            return false
        @unknown default:
            return false
        }
    }

    func start(onFrame: @escaping ([Float]) -> Void) throws {
        stop()

        guard let outputFormat = AVAudioFormat(
            commonFormat: .pcmFormatFloat32,
            sampleRate: Self.sampleRate,
            channels: Self.channels,
            interleaved: false
        ) else {
            throw AudioEngineError.converterUnavailable
        }

        self.onFrame = onFrame
        pendingSamples.removeAll(keepingCapacity: true)

        let inputNode = engine.inputNode
        let inputFormat = inputNode.outputFormat(forBus: 0)
        guard let converter = AVAudioConverter(from: inputFormat, to: outputFormat) else {
            throw AudioEngineError.converterUnavailable
        }
        self.converter = converter

        inputNode.installTap(onBus: 0, bufferSize: 4096, format: inputFormat) { [weak self] buffer, _ in
            self?.handleInputBuffer(buffer)
        }

        engine.prepare()
        try engine.start()
    }

    func stop() {
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        converter = nil
        onFrame = nil
        pendingSamples.removeAll(keepingCapacity: false)
    }

    private func handleInputBuffer(_ buffer: AVAudioPCMBuffer) {
        do {
            let samples = try convert(buffer: buffer)
            guard !samples.isEmpty else { return }

            pendingSamples.append(contentsOf: samples)
            while pendingSamples.count >= Self.frameSize {
                let frame = Array(pendingSamples.prefix(Self.frameSize))
                pendingSamples.removeFirst(Self.frameSize)
                onFrame?(frame)
            }
        } catch {
            print("AudioEngine error: \(error.localizedDescription)")
        }
    }

    private func convert(buffer: AVAudioPCMBuffer) throws -> [Float] {
        guard let converter else {
            throw AudioEngineError.converterUnavailable
        }

        let ratio = Self.sampleRate / buffer.format.sampleRate
        let estimatedFrameCount = Int((Double(buffer.frameLength) * ratio).rounded(.up)) + 32
        guard let outputBuffer = AVAudioPCMBuffer(
            pcmFormat: converter.outputFormat,
            frameCapacity: AVAudioFrameCount(max(estimatedFrameCount, Self.frameSize))
        ) else {
            throw AudioEngineError.bufferAllocationFailed
        }

        var conversionError: NSError?
        let inputState = ConverterInputState(buffer: buffer)
        let status = converter.convert(to: outputBuffer, error: &conversionError) { _, outStatus in
            if inputState.didProvideInput {
                outStatus.pointee = .noDataNow
                return nil
            }

            inputState.didProvideInput = true
            outStatus.pointee = .haveData
            return inputState.buffer
        }

        if let conversionError {
            throw AudioEngineError.conversionFailed(conversionError.localizedDescription)
        }

        if status == .error || outputBuffer.frameLength == 0 {
            return []
        }

        guard let channelData = outputBuffer.floatChannelData else {
            throw AudioEngineError.conversionFailed("Missing float channel data")
        }

        let frameCount = Int(outputBuffer.frameLength)
        return Array(UnsafeBufferPointer(start: channelData[0], count: frameCount))
    }
}

private final class ConverterInputState: @unchecked Sendable {
    let buffer: AVAudioPCMBuffer
    var didProvideInput = false

    init(buffer: AVAudioPCMBuffer) {
        self.buffer = buffer
    }
}
