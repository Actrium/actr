import ActrBindings
import Foundation
import Network

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

    mutating func didEnterBackground(at now: Date) -> AppLifecycleState? {
        guard backgroundedAt == nil else {
            return nil
        }
        backgroundedAt = now
        return .background
    }

    mutating func willEnterForeground(at now: Date) -> AppLifecycleState {
        guard let backgroundedAt else {
            return .foreground(backgroundDurationMs: 0)
        }

        self.backgroundedAt = nil
        let elapsedMs = max(0, now.timeIntervalSince(backgroundedAt) * 1_000)
        return .foreground(backgroundDurationMs: UInt64(elapsedMs.rounded()))
    }
}
