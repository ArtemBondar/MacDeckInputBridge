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
