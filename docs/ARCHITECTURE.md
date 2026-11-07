# Architecture

MacBook keyboard → Swift `CGEventTap` → authenticated encrypted TCP → Rust receiver → Linux `/dev/uinput` → KDE Plasma Desktop Mode.

## States

- **LOCAL** passes ordinary input through unchanged.
- **CONNECTING** establishes the TCP session without suppressing any local event.
- **REMOTE** suppresses only mapped ordinary key events on macOS and forwards their Linux evdev code plus press/release/repeat state.
- A connection error, disconnect, or event-tap failure returns to **LOCAL** immediately. The receiver releases every injected held key whenever a client session ends.

## Hotkey and modifiers

`Control + Shift + F12` (macOS virtual keycode 111) starts connecting from LOCAL and returns to LOCAL from REMOTE. The hotkey itself always passes locally and is never sent. Quartz `flagsChanged` events generate modifier transitions. Modifiers held before REMOTE is entered retain local delivery for their later key-up, preventing stuck local modifiers during a partial hotkey sequence. macOS-reserved shortcuts cannot be guaranteed to be intercepted.

## Protocol

The receiver first writes an unencrypted, length-framed `ServerHello(version, 32-byte challenge)`. Both peers derive a one-connection 32-byte AEAD key using HKDF-SHA256 with the pairing secret as input key material, the challenge as salt, and `MacDeckInputBridge/session/v1` as info. The sender must send encrypted `ClientReady` at sequence 0; the receiver returns encrypted `ServerReady` at sequence 0. Only after that confirmation may the sender enter REMOTE.

Application records are a four-byte big-endian bounded length followed by a ChaCha20-Poly1305 combined box: a fresh 12-byte nonce, ciphertext, and 16-byte tag. Its authenticated plaintext begins with a strictly increasing big-endian `u64` sequence number, then `Key(code, value)`, `ReleaseAll`, or `Ping`; values are 0 (up), 1 (down), and 2 (repeat). The receiver rejects malformed, unauthenticated, duplicate, and out-of-order records before injection. The fresh session key prevents a captured record being replayed on another TCP connection.

The sender maps physical macOS virtual key codes to Linux `KEY_*`/evdev codes; it does not derive, log, or persist typed Unicode content.

## Security and privileges

Keyboard events can contain credentials. Keep the pairing secret confidential, use a trusted Wi-Fi network, and do not expose the receiver port to the Internet. The receiver accepts only an authenticated ready exchange and in-order session records. Its process should run as an ordinary user with group access to `/dev/uinput`, rather than as root; see [SteamOS setup](steamos-setup.md).

The listener accepts connections without blocking on a handshake. At most four unauthenticated handshakes may be pending, each with a five-second timeout, and only one authenticated peer may own the virtual keyboard. The Desktop Mode app asks the receiver to stop with `SIGTERM`; the receiver polls for shutdown, releases all held virtual keys, and then destroys its uinput device. A forced termination is a bounded fallback only.

## Acceptance

1. The hotkey moves safely between LOCAL, CONNECTING, and REMOTE.
2. Letters, punctuation, navigation, function keys, and modifiers work in KDE Plasma.
3. REMOTE input does not duplicate eligible ordinary keystrokes on macOS.
4. Wi-Fi loss restores local input and clears remote held keys.
5. Disconnect while a modifier is held leaves no stuck key.
6. Receiver termination and macOS event-tap timeout fail safe.
7. Physical MacBook and Steam Deck validation follows the [manual checklist](manual-verification.md); CI alone is insufficient.
