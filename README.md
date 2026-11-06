# MacDeckInputBridge

An MVP keyboard bridge from a MacBook to a Steam Deck OLED running Desktop Mode. The macOS sender captures physical keyboard events through an active Quartz event tap; the SteamOS receiver injects Linux key events through `/dev/uinput`.

It intentionally does not provide Bluetooth, a GUI, trackpad forwarding, typed-content logs, or persistence of typed content. It also does not claim to suppress macOS-reserved system shortcuts.

## Pair and run

Build the receiver on the Steam Deck:

```sh
cd receiver
cargo build --release
./target/release/mac-deck-receiver pair
```

Transfer the printed 32-byte Base64 secret over a trusted channel. Treat it like a password: anyone who has it and can reach the TCP listener can authenticate as a sender. Start the receiver without making the process root; an example least-privilege udev setup is in [`docs/steamos-setup.md`](docs/steamos-setup.md).

```sh
MAC_DECK_PAIRING_SECRET='paste-secret-here' \
  ./target/release/mac-deck-receiver serve --bind 0.0.0.0:4242
```

Build and run the sender on macOS:

```sh
swift build
.build/debug/mac-deck-sender --host 192.168.1.42 --pairing-secret 'paste-secret-here'
```

The sender requests Accessibility permission. With the sender running, `Control + Shift + F12` starts connecting. Input remains local while connecting. Once authenticated, the same hotkey returns immediately to LOCAL mode and releases every remotely held key. On some MacBooks, use `Fn + Control + Shift + F12`.

## Protocol and security model

Each TCP record is a four-byte big-endian length followed by CryptoKit/Rust `ChaCha20-Poly1305` combined data: a fresh 12-byte nonce, ciphertext, and a 16-byte authentication tag. The paired 32-byte random secret is the AEAD key. Packets contain only an event type, Linux evdev key code, and press/release/repeat state—not text or keyboard layout output. The receiver rejects malformed or unauthenticated frames, and it releases held keys for every connection termination or error.

Pairing is explicit: generate a secret on the receiver, transfer it out-of-band, and pass the same secret to the sender. This protects against network observation/tampering only while the secret remains confidential. Use a private Wi-Fi network, do not expose port 4242 to the Internet, and generate a new secret after any suspected disclosure.

## Development

```sh
swift test
(cd receiver && cargo test)
```

Run the physical-device checklist before relying on the bridge: [`docs/manual-verification.md`](docs/manual-verification.md).
