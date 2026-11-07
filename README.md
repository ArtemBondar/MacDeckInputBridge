# MacDeckInputBridge

An MVP keyboard bridge from a MacBook to a Steam Deck OLED running Desktop Mode. The macOS sender captures physical keyboard events through an active Quartz event tap; the SteamOS receiver injects Linux key events through `/dev/uinput`.

It intentionally does not provide Bluetooth or trackpad forwarding, typed-content logs, or persistence of typed content. It also does not claim to suppress macOS-reserved system shortcuts.

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

### Steam Deck Desktop Mode app

The native receiver app provides the receiver through a **Control** toggle and a **Settings** tab. It displays and copies the pairing secret, generates a replacement secret, and lists the Deck's local IPv4 addresses. Generating a replacement key first disables Mac input and tears down the virtual keyboard; enable it again only after updating the Mac sender with the new key.

Build both binaries together so the app can launch its CLI receiver sibling:

```sh
cd receiver
cargo build --release --locked --features desktop-ui --bins
sh scripts/install-steamdeck-app.sh
```

This installs **MacDeck** into the Steam Deck Desktop Mode application launcher.

The installer places both sibling binaries in `~/.local/bin`, gives the launcher an
absolute path to its UI binary (so Plasma does not depend on its `PATH`), and
installs the MacDeck icon. Run it again after building a newer version. It also
asks Plasma to refresh its application cache; if the app is not visible right
away, log out and back in. A full checkout installs the exact macOS app icon;
source-only archives use the matching Steam Deck silhouette instead. The
installer also registers the asset through `xdg-icon-resource` for SteamOS
Plasma versions that do not immediately discover user theme files. The same
Dock PNG is embedded in the receiver window and its Wayland app ID matches the
launcher, so the Plasma task manager groups it with the correct icon.

The app stores only its listen address and pairing secret in `$XDG_CONFIG_HOME/mac-deck-input-bridge/receiver.toml` (or `~/.config/...`), with owner-only permissions. It never records forwarded key events or typed content. The receiver still needs the non-root `/dev/uinput` setup described in [`docs/steamos-setup.md`](docs/steamos-setup.md).

#### Optional Mac-first layout synchronization

In the Deck app’s **Settings** tab, enable **Sync active layout from Mac**. The
receiver automatically matches the active Mac source ID against the configured
Plasma/XKB layouts and changes the Deck only when the match is unambiguous.
Optional manual overrides use one mapping per input source, for example:

```text
com.apple.keylayout.US = 0
com.apple.keylayout.Ukrainian = 1
```

The number is the zero-based order of that layout in **Plasma Settings →
Keyboard → Layouts**. This is deliberately data-driven rather than limited to a
small language list: use an override only for a custom or ambiguous layout. The
macOS app shows the current source ID in its **Settings** window. When a source
cannot be matched or Plasma cannot switch it, physical-key forwarding continues
using the Deck’s current layout.

Build and run the sender on macOS:

```sh
swift build
.build/debug/mac-deck-sender --host 192.168.1.42 --pairing-secret 'paste-secret-here'
```

The sender requests Accessibility permission. With the sender running, `Control + Shift + F12` starts connecting. Input remains local while connecting. Once authenticated, the same hotkey returns immediately to LOCAL mode and releases every remotely held key. On some MacBooks, use `Fn + Control + Shift + F12`.

### macOS menu-bar app

For development, run:

```sh
swift run mac-deck-menu-bar
```

For normal use, build and launch a real macOS app bundle instead of launching
the executable from Terminal. This ensures Settings can become the foreground
application and receive keyboard focus:

```sh
sh scripts/build-macos-app.sh
open .build/MacDeckInputBridge.app
```

To install it into your user Applications folder with its Dock icon, run:

```sh
sh scripts/install-macos-app.sh
```

Then open **MacDeckInputBridge** from Finder’s Applications folder and, if you
want, use **Options → Keep in Dock** from its Dock icon.

Grant **MacDeckInputBridge** Accessibility access only after building this app
bundle; do not grant access to the `swift run` process. If macOS still prompts
after a rebuild, quit the app, turn its **MacDeckInputBridge** switch off and
back on once in **System Settings → Privacy & Security → Accessibility**, then
reopen `.build/MacDeckInputBridge.app`.

### Keep Accessibility access across development builds

macOS identifies privacy permissions through the app's code-signing designated
requirement. An ad-hoc signature is tied to one exact build, so it causes an
Accessibility prompt after source changes. Before repeatedly rebuilding, add
an **Apple Development** certificate in **Xcode → Settings → Accounts → Manage
Certificates → + → Apple Development**. The build script automatically uses
that identity (or use `MAC_DECK_CODESIGN_IDENTITY='Apple Development: …' sh
scripts/install-macos-app.sh` to select one). Grant Accessibility once to the
resulting app bundle; later builds signed by that same identity retain it.

Use **Settings…** to enter the Steam Deck host, port, and pairing secret. The secret is stored in the Mac Keychain, not in preferences or logs. **Show MacDeckInputBridge in Dock** is on by default for a consistent foreground Settings experience; turn it off to use the app exclusively from the menu bar. The larger menu-bar Steam Deck silhouette uses its central screen to show status: an empty outlined screen for **LOCAL**, animated zero-to-three dots while **CONNECTING**, and `ABC` only while the keyboard is captured by the **Steam Deck**. The menu also exposes the same `Control + Shift + F12` toggle.

## Protocol and security model

Each connection begins with a fresh 32-byte receiver challenge. Both peers use HKDF-SHA256 to derive a per-connection key from that challenge and the paired 32-byte secret, then exchange authenticated `ClientReady`/`ServerReady` records. Only then can macOS enter REMOTE mode. Every later TCP record is length-framed and encrypted with CryptoKit/Rust `ChaCha20-Poly1305`; its authenticated plaintext contains a strictly increasing 64-bit sequence number plus a key event. A captured record cannot be replayed in a different session, or twice within one session. Packets contain only an event type, Linux evdev key code, and press/release/repeat state—not text or keyboard layout output. The receiver rejects malformed, unauthenticated, replayed, or out-of-order frames, and releases held keys for every connection termination or error.

Pairing is explicit: generate a secret on the receiver, transfer it out-of-band, and pass the same secret to the sender. This protects against network observation/tampering only while the secret remains confidential. Use a private Wi-Fi network, do not expose port 4242 to the Internet, and generate a new secret after any suspected disclosure.

## Development

```sh
swift test
(cd receiver && cargo test --locked --features desktop-ui)
```

`swift test` also runs AppKit UI automation for the macOS Settings window: it verifies that the window becomes key, the Host field accepts typed text, the pairing field accepts paste, and Save receives the entered values.

The UI scenario matrix and contribution rule are in [`docs/UI_TESTING.md`](docs/UI_TESTING.md).

The future signed GitHub release workflow is deliberately deferred until hardware testing is complete; its approval-gated plan is in [`docs/RELEASE_PLAN.md`](docs/RELEASE_PLAN.md).

Run the physical-device checklist before relying on the bridge: [`docs/manual-verification.md`](docs/manual-verification.md).
