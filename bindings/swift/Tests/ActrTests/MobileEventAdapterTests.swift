@testable import Actr
import ActrBindings
import Foundation
import Network
import Testing

private func transport(
    wifi: Bool = false,
    cellular: Bool = false,
    ethernet: Bool = false,
    vpn: Bool = false,
    other: Bool = false
) -> NetworkTransportFlags {
    NetworkTransportFlags(
        wifi: wifi,
        cellular: cellular,
        ethernet: ethernet,
        vpn: vpn,
        other: other
    )
}

@Test func networkPathReducerSuppressesInitialAndDuplicateObservations() {
    var reducer = NetworkPathEventReducer()
    let wifi = NetworkPathObservation(
        status: .satisfied,
        transport: transport(wifi: true),
        isExpensive: false,
        isConstrained: false
    )

    let initial = reducer.reduce(wifi)
    #expect(initial.isInitial)
    #expect(!initial.shouldNotify)
    #expect(initial.snapshot.sequence == 1)
    #expect(initial.snapshot.availability == .available)

    let duplicate = reducer.reduce(wifi)
    #expect(!duplicate.isInitial)
    #expect(!duplicate.shouldNotify)
    #expect(duplicate.snapshot.sequence == 2)

    let forced = reducer.reduce(wifi, forceNotify: true)
    #expect(forced.shouldNotify)
    #expect(forced.snapshot.sequence == 3)
}

@Test func networkPathReducerEmitsEveryMaterialSnapshotField() {
    var reducer = NetworkPathEventReducer()
    _ = reducer.reduce(
        NetworkPathObservation(
            status: .satisfied,
            transport: transport(wifi: true),
            isExpensive: false,
            isConstrained: false
        )
    )

    let observations = [
        NetworkPathObservation(
            status: .satisfied,
            transport: transport(cellular: true),
            isExpensive: true,
            isConstrained: false
        ),
        NetworkPathObservation(
            status: .satisfied,
            transport: transport(ethernet: true),
            isExpensive: false,
            isConstrained: false
        ),
        NetworkPathObservation(
            status: .satisfied,
            transport: transport(vpn: true, other: true),
            isExpensive: false,
            isConstrained: true
        ),
        NetworkPathObservation(
            status: .requiresConnection,
            transport: transport(),
            isExpensive: false,
            isConstrained: false
        ),
        NetworkPathObservation(
            status: .unsatisfied,
            transport: transport(),
            isExpensive: false,
            isConstrained: false
        ),
    ]

    let reductions = observations.map { reducer.reduce($0) }
    #expect(reductions.allSatisfy { $0.shouldNotify })
    #expect(reductions.map(\.snapshot.sequence) == [2, 3, 4, 5, 6])
    #expect(reductions[0].snapshot.transport.cellular)
    #expect(reductions[0].snapshot.isExpensive)
    #expect(reductions[1].snapshot.transport.ethernet)
    #expect(reductions[2].snapshot.transport.vpn)
    #expect(reductions[2].snapshot.transport.other)
    #expect(reductions[2].snapshot.isConstrained)
    #expect(reductions[3].snapshot.availability == .unknown)
    #expect(reductions[4].snapshot.availability == .unavailable)
}

@Test func lifecycleReducerPreservesFirstBackgroundTimestampAndClampsClockRollback() {
    var reducer = AppLifecycleEventReducer()
    let epoch = Date(timeIntervalSince1970: 1_000)

    #expect(reducer.willEnterForeground(at: epoch) == .foreground(backgroundDurationMs: 0))
    #expect(reducer.didEnterBackground(at: epoch) == .background)
    #expect(reducer.didEnterBackground(at: epoch.addingTimeInterval(10)) == nil)
    #expect(
        reducer.willEnterForeground(at: epoch.addingTimeInterval(60))
            == .foreground(backgroundDurationMs: 60_000)
    )

    #expect(reducer.didEnterBackground(at: epoch.addingTimeInterval(120)) == .background)
    #expect(
        reducer.willEnterForeground(at: epoch.addingTimeInterval(119))
            == .foreground(backgroundDurationMs: 0)
    )
}
