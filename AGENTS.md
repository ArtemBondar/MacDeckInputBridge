# MacDeckInputBridge — Codex instructions

Build a keyboard bridge from a MacBook to Steam Deck OLED in Desktop Mode.

## MVP
- Swift macOS CLI using active Quartz CGEventTap; Rust SteamOS CLI using Linux /dev/uinput.
- Wi-Fi TCP transport; no Bluetooth, GUI, or trackpad in MVP.
- Global toggle: Control + Shift + F12 (macOS F12 keycode 111). On some Macs Fn is required.
- LOCAL mode passes ordinary input through. REMOTE mode suppresses eligible local keyboard events and forwards key-down/up and modifier changes.
- Hotkey must remain local and never be forwarded. Handle key repeat, flagsChanged, partial hotkey sequences, and key-up state.
- CONNECTING must not suppress local typing; on disconnect or error return immediately to LOCAL.
- Receiver releases all held keys on disconnect; ensure no stuck modifiers.
- Do not claim suppression of macOS-reserved system shortcuts.
- Use authenticated, encrypted transport with explicit pairing before forwarding keystrokes; do not log or persist typed content.
- Prefer a least-privilege uinput configuration over running the whole receiver as root.
- Provide Swift Package Manager and Cargo builds, unit tests, CI, and a real-device manual verification checklist.
- Implement in reviewable steps; test framing, mapping, hotkey transitions and fail-safe behavior.
- Do not change repository visibility or add a license without the owner's instruction.

## Milestones
1. Define transport, key mapping and threat model.
2. Implement Rust receiver and tests.
3. Implement Swift event capture and state machine.
4. End-to-end secure connection and fail-safe behavior.
5. macOS and SteamOS hardware testing.

## Durable delivery rules

These rules exist because CI and desktop development have previously hidden
Steam Deck regressions. Follow them for every change that affects packaging,
input, or UI.

1. **Do not infer hardware behavior.** Treat a real-device report, photo, or
   log as authoritative. Do not claim Steam Input, Gaming Mode, accessibility,
   focus, scrolling, or device permissions work until the relevant manual
   checklist step has passed on the target hardware.
2. **Test the installed artifact, not only the checkout.** A source installer
   must build the checked-out revision before copying binaries; a release
   archive must install only its bundled binaries. Add an automated installer
   regression test whenever either path changes. Make the running build
   identifiable when practical.
3. **Keep SteamOS builds stock-compatible.** Before adding a Rust dependency,
   check whether it needs system development packages, a daemon, root, or
   `/dev/input` access. CI packages do not make that dependency available on a
   stock Steam Deck. Prefer self-contained dependencies; otherwise document,
   test, and obtain explicit approval for the SteamOS setup.
4. **Build responsive UI as a baseline.** Any content that can exceed the
   viewport must be scrollable with mouse/touch in Desktop Mode. Verify
   constrained window sizes and 1280×800 Gaming Mode separately; do not make
   scrolling contingent on controller focus.
5. **Controller support needs an end-to-end input-path test.** Do not assume
   Steam translates controller buttons to keyboard events. Verify the actual
   Game Mode event path, D-pad, stick, activation, text-entry/OSK, focus
   visibility, and scrolling on a real Deck. Maintain unit tests for mappings
   and a hardware checklist for the integration.
6. **Make failure safe and observable.** Optional UI input features must fail
   closed to normal keyboard/mouse operation, never require the app to run as
   root, never log typed text or secrets, and must provide an actionable
   non-secret diagnostic when a required capability is unavailable.
7. **Release only reviewable evidence.** Use a feature branch and PR; run the
   full locked Swift/Rust/test-installer suite. Record what CI proves and what
   still requires physical verification in the PR and release notes.
