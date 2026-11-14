# Real-device manual verification

Perform this checklist on a MacBook and Steam Deck OLED in Desktop Mode before everyday use.

## Desktop Mode system tray

- Launch the installed MacDeck app and confirm its icon appears in Plasma's system tray (including the hidden-items area). Check the tooltip and menu say whether Mac input is enabled; this state does not assert a Mac is connected.
- Use the tray menu to enable and disable input, send clipboard text, open Settings, and reopen the Control window. Verify errors are visible and no action exposes clipboard contents or the pairing secret.
- Close the window and confirm MacDeck remains available in the tray and an enabled receiver remains active. Quit from the tray and confirm the receiver exits and held keys are released.
- Confirm the close button and titlebar minimize both hide the window and remove its taskbar entry while leaving the tray available. Select Settings or activate the tray icon and confirm the window returns to the foreground. Check that disabled input has the plain symbolic icon and enabled input shows ABC; the icon indicates receiver state, not an established connection. Verify legibility in light and dark Plasma themes.
- Repeat on a desktop without an X11 display: closing must quit rather than claim the Wayland window was hidden. Test clipboard sharing while the desktop window uses XWayland.
- While the window is hidden, restart or disable Plasma's system tray. Confirm MacDeck restores its window, remains controllable, and does not leave input stuck. Repeat with no tray available at startup: closing the window must quit normally.
- While hidden, trigger a tray input error and an asynchronous clipboard error. Confirm Plasma shows a notification with actionable text and no clipboard contents or secret, without opening the window. Repeat with desktop notifications unavailable and verify the window becomes visible with the error instead.
- Verify Gaming Mode separately: no tray is expected, and the app's normal focus and close behavior must continue working.

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

# Bidirectional clipboard verification

- In Desktop Mode, enable both directions on both devices and reconnect. Verify no clipboard is sent on connection.
- Copy new plain text on each device in turn and paste it on the other. Verify no repeated echo; repeat with Unicode and text near the size limit.
- Copy before connecting, then use each device's explicit send action and verify delivery status.
- Disable each direction, disconnect and reconnect, and verify that old clipboard values are not replayed and normal keyboard input continues.
- Exercise simultaneous changes, unsupported formats, oversized text and a missing compositor capability. Verify errors do not include clipboard text or secrets.
- Test Gaming Mode separately and record the actual result; Desktop Mode results do not establish Gaming Mode support.
