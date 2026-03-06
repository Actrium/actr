import SwiftUI
import Observation

@Observable
final class AudioCaptureViewModel {
    static let autoStopInterval: Duration = .seconds(5)

    var isConnected = false
    var isRecording = false
    var statusMessage = "Not connected"
    var sampleCount: Int = 0

    private var workload: AudioCaptureWorkload?
    private var autoStopTask: Task<Void, Never>?

    @MainActor
    func connect() async {
        statusMessage = "Connecting..."
        do {
            let wl = try await AudioCaptureWorkload()
            self.workload = wl
            isConnected = true
            statusMessage = "Connected, ready to record"
        } catch {
            statusMessage = "Connection failed: \(error.localizedDescription)"
        }
    }

    @MainActor
    func startRecording() async {
        guard let workload else { return }
        do {
            try await workload.startCapture()
            isRecording = true
            statusMessage = "Recording... Auto stop in 5s"
            scheduleAutoStop()
        } catch {
            statusMessage = "Start failed: \(error.localizedDescription)"
        }
    }

    @MainActor
    func stopRecording() async {
        autoStopTask?.cancel()
        autoStopTask = nil

        guard let workload else { return }
        do {
            try await workload.stopCapture()
            isRecording = false
            statusMessage = "Stopped"
        } catch {
            statusMessage = "Stop failed: \(error.localizedDescription)"
        }
    }

    @MainActor
    private func scheduleAutoStop() {
        autoStopTask?.cancel()
        autoStopTask = Task { [weak self] in
            do {
                try await Task.sleep(for: Self.autoStopInterval)
            } catch {
                return
            }

            guard !Task.isCancelled else { return }
            await self?.autoStopRecording()
        }
    }

    @MainActor
    private func autoStopRecording() async {
        guard isRecording else { return }

        await stopRecording()
        if statusMessage == "Stopped" {
            statusMessage = "Stopped automatically after 5s"
        }
    }
}

struct ContentView: View {
    @State private var viewModel = AudioCaptureViewModel()

    var body: some View {
        VStack(spacing: 20) {
            Text("Audio Capture")
                .font(.title)

            Text(viewModel.statusMessage)
                .foregroundStyle(.secondary)

            if !viewModel.isConnected {
                Button("Connect") {
                    Task { await viewModel.connect() }
                }
            } else if viewModel.isRecording {
                Button("Stop Recording") {
                    Task { await viewModel.stopRecording() }
                }
                .tint(.red)
            } else {
                Button("Start Recording") {
                    Task { await viewModel.startRecording() }
                }
                .tint(.green)
            }
        }
        .padding(40)
        .frame(minWidth: 300, minHeight: 200)
    }
}
