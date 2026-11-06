# SteamOS receiver setup

`mac-deck-receiver` needs access to `/dev/uinput` to create a virtual keyboard. It does **not** need to run as root. Create a dedicated group and grant that group access to the device:

```sh
sudo groupadd --system uinput
sudo usermod -aG uinput "$USER"
sudo tee /etc/udev/rules.d/99-mac-deck-input-bridge.rules >/dev/null <<'RULE'
KERNEL=="uinput", MODE="0660", GROUP="uinput", OPTIONS+="static_node=uinput"
RULE
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=misc --action=change
```

Log out and back in before starting the receiver so the group membership takes effect. Verify with `test -r /dev/uinput -a -w /dev/uinput`. The receiver advertises only the keyboard capabilities mapped by the sender.

For a persistent service, put `MAC_DECK_PAIRING_SECRET=...` in a root-owned `0600` environment file and reference it from your user service. Do not put the secret in a world-readable unit file or shell history.
