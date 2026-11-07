# UI testing matrix

The GUI test suites exercise view-model and native-control behaviour without
requiring a real Steam Deck uinput device or a paired Mac. Hardware validation
still follows the [manual checklist](manual-verification.md).

## macOS menu-bar app

`swift test` verifies the AppKit Settings window becomes the foreground/key
window, accepts typed host input, accepts a pasted pairing secret, and passes
the entered values to Save. It also verifies the Dock preference defaults to
shown, persists an explicit choice, keeps an accessory Settings window usable
when Dock visibility is off, and covers the `LOCAL`, `CONNECTING`, and
`REMOTE` variants of the Steam Deck `ABC` menu-bar status icon. Core tests
cover the connection state machine, hotkey, mapping, pairing/session protocol,
replay rejection, and safe LOCAL fallbacks.

## Steam Deck receiver app

`cargo test --locked --features desktop-ui` uses a fake receiver supervisor to
cover these reachable UI states without opening `/dev/uinput` or starting a
real listener:

| Area | Covered transitions and failures |
| --- | --- |
| Control | start, stop, stale/not-running state, start failure, stop failure |
| Settings | valid/invalid endpoint and secret, disabled save, active stop-save-start, save failure, restart failure |
| Pairing | copy confirmation without exposing the secret, rotate after stop, rotate stop failure, rotate persistence failure and rollback |
| Configuration | missing, malformed, persisted, and permission-restricted files; owner-only modes |
| Network display | loopback filtering, deduplication, deterministic ordering, empty result |
| Installation | receiver sibling binary missing/present |

## Rule for new UI features

Every new visible control, status, validation rule, or state transition must
ship with an automated success test and relevant failure-path test. Tests must
use fake processes/services when the real dependency needs elevated privilege,
hardware, a network peer, or a graphical desktop session. Add the scenario to
this matrix and keep the manual checklist only for behaviour that cannot be
reproduced safely in automation.
