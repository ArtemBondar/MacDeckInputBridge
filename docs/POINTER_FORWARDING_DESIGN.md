# Pointer forwarding: implementation plan

Issue [#91](https://github.com/ArtemBondar/MacDeckInputBridge/issues/91) proposes optional trackpad forwarding in Steam Deck Desktop Mode. This document records the compatibility and safety work needed before the feature is enabled. Pointer forwarding is **not implemented** by this plan.

## Current constraints

- The sender's active Quartz event tap currently captures keyboard events only. The input state machine changes to REMOTE only after the encrypted ready exchange; timeout, tap failure, toggle-off and disconnection return to LOCAL.
- Server-ready packets encode the existing layout and clipboard capability combinations. A current sender rejects unknown ready packets; a current receiver rejects unknown client packets. Simply adding a pointer capability to the ready response, or probing it after connecting, would break one of the mixed-version combinations.
- The receiver currently creates one uinput keyboard and tracks held keys for release on session exit. It does not create a mouse. A virtual mouse must have its own button tracking and teardown.
- The sender serializes packets over the existing authenticated, encrypted TCP connection. Pointer motion may arrive at a much higher rate than keyboard events, so an unbounded motion queue could delay key releases or disconnect handling.

## Proposed delivery

1. **Protocol and compatibility.** Define bounded, validated relative-motion, button and scroll records and cross-language test vectors. Negotiate pointer support without sending unrecognized packets to an old peer. One candidate is a versioned client-ready attempt followed by a single explicit reconnect in legacy keyboard-only mode when a legacy receiver rejects it; an old client continues to use its existing ready exchange. Confirm handshake failures and downgrade behavior cannot loop, remain in CONNECTING throughout fallback, and report pointer unavailability without claiming it is active. Review this approach against connection timing and authentication before choosing the final wire format. Never interpret an unauthenticated response as permission to forward input.
2. **Receiver.** Create a separate relative uinput mouse with left/right buttons and horizontal/vertical wheel axes, using the existing non-root uinput access. Advertise pointer capability only after successful device creation. Keep keyboard service available if mouse setup fails. Track held buttons per session; release them on explicit release, disconnect, shutdown, error and device teardown. Reject malformed, unnegotiated or out-of-range pointer records before injection.
3. **Sender.** Add an opt-in setting, off by default. Capture movement, button, drag and scroll events only when the authenticated receiver confirms support. CONNECTING and any failure must pass pointer events locally. Switch keyboard and pointer together with the existing hotkey; on toggle-off release remote buttons before teardown and restore local pointer handling. Define how button transitions already in progress at the mode boundary avoid stuck buttons or orphaned clicks on either device.
4. **Movement and flow control.** Investigate Quartz relative deltas at physical screen edges and multiple displays on real hardware before selecting any cursor association or recentering strategy. Do not assume suppressing a movement event alone allows unlimited relative motion. If cursor positioning is changed, restore it predictably on all exits and avoid feedback from synthetic events. Coalesce bounded movement under load while preserving button/scroll ordering; releases and disconnect handling must not be starved. Bound memory and fail back to LOCAL on transport failure.
5. **Scroll behavior.** Decide and document how Quartz line and pixel deltas map to Linux wheel events, including fractional accumulation, direction and sensitivity. Test both axes on the target compositor; do not assume high-resolution wheel support or user preference behavior without verifying it.

Trackpad and external mouse events may share the same Quartz event types. Confirm whether the supported capture path can distinguish them before promising trackpad-only forwarding. Pinch, three-finger gestures and Gaming Mode are outside the initial scope.

## Required verification before enabling

Automated tests should cover malformed and unnegotiated packets, signed movement boundaries, scroll accumulation, ordered button transitions, mid-drag release, disconnect and write failure, high-rate motion backpressure, and all old/new sender and receiver combinations. Test the installed artifacts as well as source builds, and run the locked Swift, Rust and installer suites required for release.

On an actual MacBook and Steam Deck in Desktop Mode under Wayland, verify movement at screen edges, clicks, drags, both scroll axes, direction and sensitivity; exercise toggle-off, receiver shutdown, Wi-Fi loss and event-tap failure during a drag. Verify normal local input while connecting, disabled and after failures. Record device, OS and app versions and results. Hardware behavior remains unverified until those checks pass.
