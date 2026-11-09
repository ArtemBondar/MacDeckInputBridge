# MacDeck for Steam Deck

MacDeck receives encrypted keyboard input sent from the MacDeckInputBridge menu
bar app on a Mac. It runs in Steam Deck **Desktop Mode** and creates a virtual
keyboard only while **Mac input** is enabled in its Control tab.

## Install

1. Download and unpack the `mac-deck-input-bridge-steamdeck-x86_64.tar.gz`
   release archive.
2. In Konsole, enter the unpacked folder and run:

   ```sh
   sh receiver/scripts/install-steamdeck-app.sh
   ```

3. Open **MacDeck** from the Desktop Mode application launcher.

For a source checkout, the installer builds the current source automatically;
for a release archive, it installs its prebuilt binaries.

The receiver needs permission to write `/dev/uinput`, but does not need to run
as root. Configure this once, then log out of Desktop Mode and back in:

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

## Use

1. In MacDeck, open **Settings**.
2. Generate or copy the pairing secret and note the shown Steam Deck IP address.
3. Enter that IP address and secret in the Mac menu bar app's Settings.
4. In MacDeck's **Control** tab, choose **Enable Mac Input**.
5. On the Mac, use Control–Shift–F12 to send or release the keyboard.

Keep the pairing secret private. The Mac and Steam Deck must be on a trusted
network.

## Uninstall

From the unpacked release folder, run:

```sh
sh receiver/scripts/uninstall-steamdeck-app.sh
```

The uninstaller stops MacDeck, removes its app launcher, binaries, and icons,
then refreshes Plasma's cache. It preserves pairing settings so reinstalling
does not require pairing again. To erase them too, remove
`~/.config/mac-deck-input-bridge` manually.
