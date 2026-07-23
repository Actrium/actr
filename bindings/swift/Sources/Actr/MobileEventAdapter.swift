import ActrBindings
import Foundation
import Network

protocol MobileEventSending: Sendable {
    func sendNetworkPathChanged(snapshot: NetworkSnapshot) async
    func sendAppLifecycleChanged(state: AppLifecycleState) async
}

private final class BindingMobileEventSender: MobileEventSending, @unchecked Sendable {
    private let handle: NetworkEventHandleWrapper

    init(handle: NetworkEventHandleWrapper) {
        self.handle = handle
    }

    func sendNetworkPathChanged(snapshot: NetworkSnapshot) async {
        _ = try? await handle.handleNetworkPathChanged(snapshot: snapshot)
    }

    func sendAppLifecycleChanged(state: AppLifecycleState) async {
        _ = try? await handle.handleAppLifecycleChanged(state: state)
    }
}

final class MobileEventDeliveryGate: @unchecked Sendable {
    private let sender: any MobileEventSending
    private let stateLock = NSLock()
    private var closed = false

    convenience init(handle: NetworkEventHandleWrapper) {
        self.init(sender: BindingMobileEventSender(handle: handle))
    }

    init(sender: any MobileEventSending) {
        self.sender = sender
    }

    func close() {
        stateLock.lock()
        closed = true
        stateLock.unlock()
    }

    func sendNetworkPathChanged(snapshot: NetworkSnapshot) async {
        guard isOpen() else { return }
        await sender.sendNetworkPathChanged(snapshot: snapshot)
    }

    func sendAppLifecycleChanged(state: AppLifecycleState) async {
        guard isOpen() else { return }
        await sender.sendAppLifecycleChanged(state: state)
    }

    private func isOpen() -> Bool {
        stateLock.lock()
        defer { stateLock.unlock() }
        return !closed
    }
}

struct NetworkPathObservation: Equatable {
    let status: NWPath.Status
    let transport: NetworkTransportFlags
    let isExpensive: Bool
    let isConstrained: Bool

    var availability: NetworkAvailability {
        switch status {
        case .satisfied:
            return .available
        case .unsatisfied:
            return .unavailable
        case .requiresConnection:
            return .unknown
        @unknown default:
            return .unknown
        }
    }
}

struct NetworkPathReduction {
    let snapshot: NetworkSnapshot
    let isInitial: Bool
    let shouldNotify: Bool
}

struct NetworkPathEventReducer {
    private var lastObservation: NetworkPathObservation?
    private var nextSequence: UInt64 = 1

    mutating func reduce(
        _ observation: NetworkPathObservation,
        forceNotify: Bool = false
    ) -> NetworkPathReduction {
        let snapshot = NetworkSnapshot(
            sequence: nextSequence,
            availability: observation.availability,
            transport: observation.transport,
            isExpensive: observation.isExpensive,
            isConstrained: observation.isConstrained
        )
        nextSequence += 1

        let isInitial = lastObservation == nil
        let shouldNotify = forceNotify || (!isInitial && lastObservation != observation)
        lastObservation = observation

        return NetworkPathReduction(
            snapshot: snapshot,
            isInitial: isInitial,
            shouldNotify: shouldNotify
        )
    }
}

struct AppLifecycleEventReducer {
    private var backgroundedAt: Date?
    private var hasLifecycleObservation = false

    mutating func initializePhase(isBackground: Bool, at now: Date) -> AppLifecycleState? {
        guard !hasLifecycleObservation else {
            return nil
        }
        hasLifecycleObservation = true
        if isBackground {
            backgroundedAt = now
            return .background
        }
        backgroundedAt = nil
        return .foreground(backgroundDurationMs: 0)
    }

    mutating func didEnterBackground(at now: Date) -> AppLifecycleState? {
        hasLifecycleObservation = true
        guard backgroundedAt == nil else {
            return nil
        }
        backgroundedAt = now
        return .background
    }

    mutating func willEnterForeground(at now: Date) -> AppLifecycleState {
        hasLifecycleObservation = true
        guard let backgroundedAt else {
            return .foreground(backgroundDurationMs: 0)
        }

        self.backgroundedAt = nil
        let elapsedMs = max(0, now.timeIntervalSince(backgroundedAt) * 1_000)
        return .foreground(backgroundDurationMs: UInt64(elapsedMs.rounded()))
    }
}
