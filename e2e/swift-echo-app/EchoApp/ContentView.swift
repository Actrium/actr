import SwiftUI

struct ContentView: View {
    @StateObject private var actrService = ActrService()
    @State private var input = ProcessInfo.processInfo.environment["ACTR_ECHOAPP_TEST_INPUT"] ?? "hello"
    @State private var output = ""
    @State private var isSending = false

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("EchoApp")
                .font(.largeTitle.bold())

            Text(actrService.status)
                .font(.footnote)
                .foregroundStyle(actrService.isReady ? Color.green : Color.secondary)

            TextField("Message", text: $input)
                .textFieldStyle(.roundedBorder)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()

            Button {
                Task {
                    await sendEcho()
                }
            } label: {
                if isSending {
                    ProgressView()
                } else {
                    Text("Send")
                        .frame(maxWidth: .infinity)
                }
            }
            .buttonStyle(.borderedProminent)
            .disabled(!actrService.isReady || input.isEmpty || isSending)

            Text("Reply")
                .font(.headline)
            Text(output.isEmpty ? "No reply yet" : output)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
                .background(.thinMaterial)
                .clipShape(RoundedRectangle(cornerRadius: 12))

            Spacer()
        }
        .padding(24)
        .task {
            await actrService.startIfNeeded()
            if actrService.shouldAutoSend() && !input.isEmpty {
                await sendEcho()
            }
        }
    }

    private func sendEcho() async {
        isSending = true
        output = ""

        defer { isSending = false }
        do {
            output = try await actrService.sendEcho(input)
            print("ACTR_E2E_RESULT:\(output)")
        } catch {
            output = "Echo failed: \(error)"
            print("ACTR_E2E_RESULT:\(output)")
        }
    }
}
