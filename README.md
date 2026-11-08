# MacDeckInputBridge

<p align="center">
  <img src="assets/MacDeckInputBridgeIcon.png" width="256" height="256" alt="MacDeckInputBridge">
</p>

Send a Mac keyboard to a Steam Deck in Desktop Mode over your local network.
MacDeck captures physical key events on the Mac and creates a virtual keyboard
on the Deck. The connection is paired, authenticated, and encrypted.

Use it only on a trusted network. The pairing secret is a password for your
keyboard connection; do not share it or expose the Deck receiver to the
Internet.

## Install on Steam Deck

1. Switch to **Desktop Mode** and download
   `mac-deck-input-bridge-steamdeck-x86_64.tar.gz` from the
   [latest release](https://github.com/ArtemBondar/MacDeckInputBridge/releases).
2. In Konsole:

   ```sh
   cd ~/Downloads
   tar -xzf mac-deck-input-bridge-steamdeck-x86_64.tar.gz
   cd mac-deck-input-bridge-steamdeck-x86_64
   sh receiver/scripts/install-steamdeck-app.sh
   ```

3. Open **MacDeck** from the Desktop Mode application launcher.

MacDeck needs access to Linux's virtual-input device. Run this once, then log
out of Desktop Mode and back in:

```sh
sudo steamos-readonly disable
sudo groupadd --system uinput
sudo usermod -aG uinput "$USER"
sudo tee /etc/udev/rules.d/99-mac-deck-input-bridge.rules >/dev/null <<'EOF'
KERNEL=="uinput", MODE="0660", GROUP="uinput", OPTIONS+="static_node=uinput"
EOF
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=misc --action=change
sudo steamos-readonly enable
```

If the `uinput` group already exists, skip only the `groupadd` command.

## Install on macOS

1. Download and unzip `MacDeckInputBridge-macos.zip` from the same release.
2. Move `MacDeckInputBridge.app` to **Applications** and open it.
3. The current release is not notarized. On the first launch, choose **Done**
   in the warning, then open **System Settings → Privacy & Security → Open
   Anyway** and confirm **Open**. Only do this for a file downloaded from this
   repository's release page.
4. When macOS asks, allow **MacDeckInputBridge** in **Privacy & Security →
   Accessibility**. This is required to read and forward physical key events.

## Pair and use

1. In MacDeck on the Deck, open **Settings**, generate or copy the pairing
   secret, and note the displayed Deck IP address.
2. In the Mac menu-bar app, open **Settings** and enter that IP address and
   pairing secret.
3. In MacDeck's **Control** tab, choose **Enable Mac Input**.
4. On the Mac, choose **Send keyboard to Steam Deck** or press
   Control–Shift–F12. Press it again to return the keyboard to the Mac.

The Mac stores the pairing secret in the macOS Keychain, not in app preferences
or logs. macOS may ask for your account password when the app saves or updates
that secret. The prompt authorizes access to your user Keychain so other users
and ordinary preference files cannot read it.

## Keyboard layouts

MacDeck forwards physical keys, not typed characters. It works with any
keyboard layout, but the Mac and Deck need matching active layouts for letters
and symbols to produce the same text.

Enable **Sync active layout from Mac** in MacDeck's Settings to switch the
Deck layout automatically. MacDeck compares the Mac input-source identifier
with the layouts configured in **Plasma Settings → Keyboard → Layouts** and
switches only when there is one clear match.

For custom or ambiguous layouts, add a manual mapping in MacDeck. The value is
the zero-based order of the layout in Plasma, for example:

```text
com.apple.keylayout.US = 0
com.apple.keylayout.Ukrainian = 1
```

To get the value on the left, switch to that layout on the Mac, then open the
menu-bar app's **Settings…** and copy **Current Mac layout ID**.

If no match is found, key forwarding continues with the Deck's current layout.

## Uninstall from Steam Deck

From the extracted Steam Deck release folder, run:

```sh
sh receiver/scripts/uninstall-steamdeck-app.sh
```

It removes MacDeck's launcher, binaries, and icons, but keeps pairing settings
for a future reinstall. To remove those too:

```sh
rm -rf ~/.config/mac-deck-input-bridge
```

## Privacy and limits

- No typed text is logged or stored.
- The receiver runs as your Deck user; it does not run as root.
- The bridge forwards keyboard input only—no trackpad or Bluetooth support.
- macOS-reserved shortcuts may remain handled by macOS.
