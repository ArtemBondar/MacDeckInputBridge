# Real-device manual verification

Perform this checklist on a MacBook and Steam Deck OLED in Desktop Mode before everyday use.

1. Build both components from the same revision and run `swift test` plus `cargo test --locked --features desktop-ui`. The hardened sender and receiver must be updated together.
2. Generate a fresh receiver pairing secret; start the receiver as a non-root user with `/dev/uinput` access; start the sender with the Deck's Wi-Fi address.
3. Confirm ordinary local typing remains local before invoking the hotkey and while the sender is CONNECTING (including with the receiver stopped).
4. Press `Control + Shift + F12` (use `Fn` if required). Confirm the hotkey itself remains local; it must not type or trigger keys on the Deck. The menu-bar icon must remain CONNECTING until the Deck has authenticated the sender, then become REMOTE.
5. After connection, verify alphanumerics, punctuation, arrows, tab, return, delete, function keys, left/right Control, Shift, Option/Alt, and Command/Super. Check key repeat on a held key.
6. Press the hotkey again while holding Control or Shift. Confirm the Mac has no stuck modifiers and the Deck releases all held keys.
7. Turn off Wi-Fi or stop the receiver while holding a normal key and while holding each modifier. Confirm local typing returns immediately and the Deck has no stuck key or modifier.
8. Try a macOS-reserved shortcut. Do not rely on it being suppressed: reserved shortcuts can be handled by macOS before the event tap.
9. Confirm logs/terminal output contain lifecycle information only, never text typed on either machine.
