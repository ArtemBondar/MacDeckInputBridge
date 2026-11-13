# Real-device manual verification

Perform this checklist on a MacBook and Steam Deck OLED in Desktop Mode before everyday use.

1. Build both components from the same revision and run `swift test` plus `cargo test --locked --features desktop-ui`. The hardened sender and receiver must be updated together.
2. Generate a fresh receiver pairing secret; start the receiver as a non-root user with `/dev/uinput` access; start the sender with the Deck's Wi-Fi address.
3. Confirm ordinary local typing remains local before invoking the hotkey and while the sender is CONNECTING (including with the receiver stopped).
4. Press `Control + Shift + F12` (use `Fn` if required). Confirm the hotkey itself remains local; it must not type or trigger keys on the Deck. The menu-bar icon must remain CONNECTING until the Deck has authenticated the sender, then become REMOTE.
5. After connection, verify alphanumerics, punctuation, arrows, tab, return, delete, function keys, left/right Control, Shift, Option/Alt, and Command/Super. Check key repeat on a held key.
6. Press the hotkey again while holding Control or Shift. Confirm the Mac has no stuck modifiers and the Deck releases all held keys.
7. Turn off Wi-Fi or stop the receiver while holding a normal key and while holding each modifier. Confirm local typing returns immediately and the Deck has no stuck key or modifier.
8. In the Steam Deck app, disable Mac input while holding a remote modifier. Confirm the app stops promptly and the Deck has no stuck key or modifier. Repeat after changing receiver Settings while input is enabled.
9. Try a macOS-reserved shortcut. Do not rely on it being suppressed: reserved shortcuts can be handled by macOS before the event tap.
10. Confirm logs/terminal output contain lifecycle information only, never text typed on either machine.
11. In Steam Deck Gaming Mode, open MacDeck at 1280×800. Confirm the receiver state and IP address are visible on Control, then open Settings and use the D-pad to reach every control. Focus must remain visible and scroll the page to the focused item; A/Enter must activate buttons and checkboxes. With a text field focused, use Steam + X to open the on-screen keyboard.
12. In Desktop Mode, run the installed app's **Check Deck layout sync** with the normal Plasma environment. Confirm the diagnostic finds an executable qdbus and reports the configured layouts. Repeat with a non-executable qdbus earlier on PATH and a usable one later; confirm layout sync still works. In an isolated environment with no usable qdbus, confirm the diagnostic explains how to expose or configure it and ordinary keyboard forwarding remains available.
13. Launch MacDeck through Steam as a non-Steam shortcut in Desktop Mode as well as in Gaming Mode. Record whether clipboard and controller controls behave as intended in each mode; do not infer the session from Steam application IDs alone.
