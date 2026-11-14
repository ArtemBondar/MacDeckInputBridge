import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("ci-scope.py")
spec = importlib.util.spec_from_file_location("ci_scope", SCRIPT)
scope = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scope)


class ScopeTests(unittest.TestCase):
    def test_routes_known_and_shared_paths(self):
        cases = [
            ([b"Sources/MacDeckMenuBarUI/MenuBarDeckIcon.swift"], (True, False)),
            ([b"receiver/src/gui.rs"], (False, True)),
            ([b"receiver/src/lib.rs"], (True, True)),
            ([b"Sources/BridgeCore/Protocol.swift"], (True, True)),
            ([b".github/workflows/release.yml"], (True, True)),
            ([b"new/path"], (True, True)),
            ([], (True, True)),
            ([b"Sources/MacDeckMenuBar/main.swift", b"receiver/src/gui.rs"], (True, True)),
        ]
        for paths, expected in cases:
            with self.subTest(paths=paths):
                self.assertEqual(scope.classify(paths), expected)

    def test_git_rename_delete_and_invalid_base(self):
        with tempfile.TemporaryDirectory() as directory:
            cwd = Path(directory)
            def git(*args):
                return subprocess.check_output(["git", *args], cwd=cwd, stderr=subprocess.DEVNULL).decode().strip()
            git("init", "-q")
            git("config", "user.email", "test@example.invalid")
            git("config", "user.name", "CI test")
            source = cwd / "Sources/MacDeckMenuBar/a.swift"
            source.parent.mkdir(parents=True)
            source.write_text("same content\n")
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD")
            target = cwd / "receiver/src/gui.rs"
            target.parent.mkdir(parents=True)
            source.rename(target)
            git("add", "-A")
            git("commit", "-qm", "rename")
            head = git("rev-parse", "HEAD")
            output = cwd / "output"
            def run_scope(start):
                output.write_text("")
                subprocess.run(["python3", str(SCRIPT), start, head], cwd=cwd,
                               env={**os.environ, "GITHUB_OUTPUT": str(output)}, check=True)
                return output.read_text()
            self.assertEqual(run_scope(base), "swift=true\nreceiver=true\n")
            self.assertEqual(run_scope("0" * 40), "swift=true\nreceiver=true\n")
            self.assertEqual(run_scope("missing-revision"), "swift=true\nreceiver=true\n")
            output.unlink()
            target.unlink()
            git("add", "-A")
            git("commit", "-qm", "delete")
            deleted = git("rev-parse", "HEAD")
            output.write_text("")
            subprocess.run(["python3", str(SCRIPT), head, deleted], cwd=cwd,
                           env={**os.environ, "GITHUB_OUTPUT": str(output)}, check=True)
            self.assertEqual(output.read_text(), "swift=false\nreceiver=true\n")


if __name__ == "__main__":
    unittest.main()
