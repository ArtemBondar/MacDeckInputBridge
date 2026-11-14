#!/usr/bin/env python3
"""Select checks from both sides of every changed path; unknown means both."""
import os
import subprocess
import sys


def classify(paths):
    swift = receiver = False
    for path in paths:
        # The protocol implementation and its tests are cross-platform.
        if path.startswith((b"Sources/BridgeCore/", b"Tests/BridgeCoreTests/")):
            return True, True
        if path.startswith((b"Sources/MacDeckMenuBar/", b"Sources/MacDeckMenuBarUI/",
                            b"Sources/MacDeckSender/", b"Tests/MacDeckMenuBarUITests/")) \
                or path in (b"Package.swift", b"scripts/install-macos-app.sh",
                            b"scripts/build-macos-app.sh", b"scripts/test-macos-app-bundle.sh"):
            swift = True
        elif path.startswith((b"receiver/resources/", b"receiver/scripts/")) \
                or path in (b"receiver/src/gui.rs", b"receiver/src/tray.rs",
                            b"receiver/src/notification.rs", b"receiver/src/clipboard_watch.rs"):
            receiver = True
        else:
            return True, True
    return (swift, receiver) if swift or receiver else (True, True)


def main():
    base, head = sys.argv[1:]
    try:
        if not base or set(base) == {"0"} or not head:
            raise ValueError("missing comparison revision")
        output = subprocess.check_output(
            ["git", "diff", "--no-renames", "--name-only", "-z", base, head, "--"],
            stderr=subprocess.DEVNULL,
        )
        paths = output.rstrip(b"\0").split(b"\0") if output else []
        swift, receiver = classify(paths)
    except (subprocess.CalledProcessError, OSError, ValueError):
        # Unavailable push history or an invalid comparison never skips checks.
        swift = receiver = True
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as out:
        print(f"swift={str(swift).lower()}", file=out)
        print(f"receiver={str(receiver).lower()}", file=out)


if __name__ == "__main__":
    main()
