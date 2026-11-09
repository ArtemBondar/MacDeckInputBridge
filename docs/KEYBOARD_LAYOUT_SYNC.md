# Keyboard layout synchronization

Key forwarding uses physical keys. Layout synchronization is optional: it asks
Plasma to select a compatible layout before the next forwarded key is handled.

When enabled, MacDeck sends the active Mac input-source ID immediately when the
encrypted session becomes ready and whenever macOS reports a source change. The
receiver applies an exact manual override first. Otherwise it compares the ID's
name with the XKB code and description of layouts configured in Plasma.

Automatic switching happens only for one unambiguous match. If a source is
unknown or more than one Deck layout could match, MacDeck leaves the Deck's
layout unchanged and still forwards physical keys. This prevents a wrong layout
from being selected silently.

For a custom or ambiguous source, copy **Current Mac layout ID** from the Mac
menu-bar app's Settings window and map it to the zero-based order shown in
**Plasma Settings → Keyboard → Layouts**:

```text
com.apple.keylayout.Ukrainian = 1
```

There is no cross-platform identifier shared by all macOS input methods and
XKB layouts, so a manual override remains necessary for an ambiguous custom
layout. Typed text is not inspected, sent, or stored for matching.

## Deck diagnostics

In MacDeck's **Settings**, enable layout sync and choose **Check Deck layout
sync**. It shows the active Plasma layout index and every configured layout
that the receiver can inspect, without opening the keyboard receiver or sending
keys.

The same read-only check is available from a terminal:

```sh
~/.local/bin/mac-deck-receiver layout-status
```

If it reports that `qdbus` is missing, run this command and include its output
when reporting the problem:

```sh
command -v qdbus6 qdbus-qt6 qdbus
```
