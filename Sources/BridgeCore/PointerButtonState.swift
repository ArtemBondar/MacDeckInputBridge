/// Tracks local and remote button ownership across a forwarding boundary.
/// Buttons held when forwarding starts remain local until their release.
struct PointerButtonState {
    enum Action: Equatable {
        case pass
        case send(button: UInt8, pressed: Bool)
        case suppress
    }

    private var physical: Set<UInt8> = []
    private var localOwnership: Set<UInt8> = []
    private var remoteOwnership: Set<UInt8> = []
    private var suppressUntilUp: Set<UInt8> = []
    private(set) var forwarding = false

    mutating func start() {
        guard !forwarding else { return }
        localOwnership = physical.subtracting(suppressUntilUp)
        forwarding = true
    }

    /// Returns buttons that need releasing on the receiver. Their physical
    /// releases must still be suppressed locally, preventing orphaned ups.
    mutating func stop() -> [UInt8] {
        guard forwarding else { return [] }
        forwarding = false
        let releases = remoteOwnership.sorted()
        suppressUntilUp.formUnion(remoteOwnership)
        remoteOwnership.removeAll()
        return releases
    }

    mutating func handle(button: UInt8, pressed: Bool) -> Action {
        guard button <= 1 else { return .pass }
        if pressed { physical.insert(button) } else { physical.remove(button) }
        if suppressUntilUp.contains(button) {
            if !pressed { suppressUntilUp.remove(button) }
            return .suppress
        }
        if localOwnership.contains(button) {
            if !pressed { localOwnership.remove(button) }
            return .pass
        }
        if remoteOwnership.contains(button) {
            if !pressed { remoteOwnership.remove(button) }
            return forwarding ? .send(button: button, pressed: pressed) : .suppress
        }
        guard forwarding else { return .pass }
        guard pressed else { return .suppress }
        remoteOwnership.insert(button)
        return .send(button: button, pressed: true)
    }
}
