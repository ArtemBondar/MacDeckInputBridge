# Keyboard layout synchronization

Key forwarding uses physical keys. Layout synchronization is optional: it asks
Plasma to select a compatible layout before the next forwarded key is handled.

When enabled, MacDeck sends the active Mac input-source ID immediately when the
encrypted session becomes ready and whenever macOS reports a source change. The
receiver applies an exact manual override first. Otherwise it compares standard
Apple layout IDs with the XKB code and full
description of layouts configured in Plasma. When available, the standard XKB
rules description also helps match layouts when Plasma names are localized.
It does not infer a match from individual words.

Automatic switching happens only for one unambiguous match. If a source is
unknown or more than one Deck layout could match, MacDeck leaves the Deck's
layout unchanged and still forwards physical keys. This prevents a wrong layout
from being selected silently.

Some standard names and variants cannot be matched reliably across platforms.
Use a manual mapping when automatic matching does not find a clear match, or
for a custom or ambiguous source. Copy **Current Mac layout ID** from the Mac
menu-bar app's Settings window and map it to the zero-based order shown in
**Plasma Settings → Keyboard → Layouts**:

```text
com.apple.keylayout.Ukrainian = 1
```

The mapping refers to a position, so update it if you reorder layouts in Plasma.
There is no cross-platform identifier shared by all macOS input methods and
XKB layouts. Typed text is not inspected, sent, or stored for matching.

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

The receiver searches `MAC_DECK_QDBUS`, then `PATH`, then common system
locations. Candidates must be executable by the current user; an unusable
candidate is skipped. If the executable is in another location, set
`MAC_DECK_QDBUS` to its full path in the receiver's environment, for example:

```sh
MAC_DECK_QDBUS=/path/to/qdbus6 ~/.local/bin/mac-deck-receiver layout-status
```

This only selects the tool: the Plasma session bus must also be available. A
missing or unusable qdbus disables layout synchronization; keyboard forwarding
continues using the Deck's current layout.

If Plasma cannot provide its runtime layout list, the receiver falls back to
its configured layout list and XKB descriptions under
`/usr/share/X11/xkb/rules`. For installations using a different location, set
`MAC_DECK_XKB_RULES_DIR` to the absolute path of the directory containing
`base.lst` or `evdev.lst` in the receiver's environment. Unreadable or empty
files fall back to the standard directory; missing descriptions fall back to
layout codes. Runtime Plasma layout order and variant take precedence; XKB descriptions are
also consulted for canonical layout names.
