# Architecture

MacBook keyboard -> Swift CGEventTap -> authenticated TCP -> Rust receiver -> Linux uinput -> KDE Plasma Desktop Mode.

## States
- LOCAL: pass through all ordinary key events.
- CONNECTING: establish and authenticate transport; keep local input usable.
- REMOTE: consume eligible keyboard events locally and send them remotely.
- ERROR/DISCONNECTED: immediately return to LOCAL and release all keys on receiver.

## Hotkey
Control + Shift + F12. macOS F12 virtual keycode is 111. Track modifiers via flagsChanged, consume hotkey locally, debounce key repeat, and avoid leaking partial shortcut events. macOS-reserved shortcuts cannot be guaranteed to be blocked.

## Protocol
Versioned, bounded, length-prefixed messages: Hello, Authenticate, KeyDown, KeyUp, ReleaseAll, Ping, Pong, Goodbye. Choose a documented physical-key mapping (USB HID usage -> Linux KEY_* recommended), not Unicode characters. Include heartbeat and reset-on-reconnect semantics.

## Security
Keyboard events may contain passwords. Require explicit device pairing and authenticated encryption before forwarding. No key-content logging. No unauthenticated public listener. Document firewall and uinput permissions.

## Acceptance
1. Hotkey toggles LOCAL/REMOTE globally.
2. Letters, punctuation and modifiers work in KDE Plasma.
3. No duplicated ordinary keystrokes on macOS in REMOTE.
4. Wi-Fi loss restores Mac keyboard.
5. Disconnect while Shift is held does not leave stuck keys.
6. Receiver termination and macOS event tap timeout fail safe.
7. Validate on physical MacBook and Steam Deck; CI alone is insufficient.
