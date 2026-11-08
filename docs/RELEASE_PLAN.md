# Release plan

Version `0.0.1` is the first owner-approved release. The GitHub Release workflow
builds it from `main`, runs the test suites, produces platform archives and
checksums, then publishes a release pinned to the exact commit. Future work is
developed on a branch and reaches `main` only when it is ready to release.

## Release gate

1. Test the exact candidate revision on a MacBook and Steam Deck OLED using [manual verification](manual-verification.md).
2. Run CI: Swift unit/UI tests; Rust format, clippy, protocol, receiver, and GUI/view-model tests.
3. Confirm a fresh pairing, reconnect, key mapping, disconnect recovery, Settings save, and app install/uninstall paths.
4. Record the candidate commit and version in release notes.

## Versioning and assets

Use a single semantic version such as `v0.1.0` for the paired protocol-compatible applications. Every release must publish checksums and a compatibility note: macOS sender and Steam Deck receiver from the same release are required.

- `MacDeckInputBridge-macos-universal.dmg` or signed ZIP containing the `.app`.
- `mac-deck-input-bridge-steamdeck-x86_64.tar.gz` containing both Rust binaries, the `.desktop` launcher, install/uninstall scripts, setup notes, and SHA-256 checksum.
- Source archive produced by GitHub.

## macOS packaging

1. Build the `.app` using the stable bundle identifier `com.artembondar.MacDeckInputBridge` and the Dock `.icns` asset.
2. Sign it with a Developer ID Application certificate supplied by the owner. Ad-hoc signing remains for local development only; it is not a distributable trust identity.
3. Submit the archive to Apple notarization and staple the ticket.
4. Verify with `codesign --verify --deep --strict` and `spctl --assess` on a clean Mac account.
5. Package the notarized app as a ZIP or DMG. The release notes must state that Accessibility permission is required after installation.

Required future GitHub secrets (never commit them): Developer ID certificate and password, Apple ID/team/notarization credentials or App Store Connect API key, and optionally a notarization keychain profile created in CI.

## Steam Deck packaging

1. Build `mac-deck-receiver` and `mac-deck-receiver-ui` for Linux `x86_64-unknown-linux-gnu` with `--locked --features desktop-ui --bins`.
2. Package both sibling binaries, `mac-deck-input-bridge.desktop`, and a non-root installer that copies into `~/.local/bin` and `~/.local/share/applications`.
3. Keep `/dev/uinput` access separate: the installer must explain the existing least-privilege udev setup and never make the GUI run as root.
4. Test the archive on a clean SteamOS Desktop Mode user. A Flatpak/AppImage can be evaluated later, but only if it preserves the uinput privilege boundary.

## GitHub workflow

1. A push to `main` releases only when the version in `VERSION` has no tag yet;
   a `v*` tag can also explicitly start a release. The tag must match `VERSION`.
2. Build macOS and Linux artifacts in isolated jobs; lock dependencies and upload artifacts.
3. Run all tests before packaging.
4. Sign/notarize macOS only when the owner has supplied secrets and explicitly enabled that job.
5. Generate SHA-256 checksums and a GitHub Release with assets and release notes.
6. No workflow changes repository visibility.
