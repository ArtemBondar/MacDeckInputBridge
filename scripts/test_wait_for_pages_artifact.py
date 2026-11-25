import json
import subprocess
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import wait_for_pages_artifact as wait


class ArtifactVisibilityTests(unittest.TestCase):
    def response(self, *pages):
        return SimpleNamespace(stdout=json.dumps([{"artifacts": page} for page in pages]))

    def artifact(self, name, expired=False):
        return {"name": name, "expired": expired}

    @patch.object(wait.time, "sleep")
    @patch.object(wait.subprocess, "run")
    def test_delayed_visibility_across_pages(self, run, sleep):
        run.side_effect = [self.response([]), self.response([self.artifact("other")], [self.artifact("pages")])]
        self.assertTrue(wait.wait_for_artifact("owner/repo", "123", "pages"))
        self.assertEqual(run.call_count, 2)
        self.assertIn("--slurp", run.call_args.args[0])
        sleep.assert_called_once_with(wait.POLL_INTERVAL_SECONDS)

    @patch.object(wait.time, "sleep")
    @patch.object(wait.subprocess, "run")
    def test_timeout_is_bounded_and_expired_artifacts_do_not_count(self, run, sleep):
        run.return_value = self.response([self.artifact("pages", expired=True)])
        self.assertFalse(wait.wait_for_artifact("owner/repo", "123", "pages"))
        self.assertEqual(run.call_count, wait.MAX_CHECKS)
        self.assertEqual(sleep.call_count, wait.MAX_CHECKS - 1)

    @patch.object(wait.time, "sleep")
    @patch.object(wait.subprocess, "run")
    def test_duplicates_fail_immediately(self, run, sleep):
        run.return_value = self.response([self.artifact("pages")], [self.artifact("pages")])
        with self.assertRaises(ValueError):
            wait.wait_for_artifact("owner/repo", "123", "pages")
        sleep.assert_not_called()

    @patch.object(wait.time, "sleep")
    @patch.object(wait.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "gh"))
    def test_api_errors_are_not_misreported_as_visibility_timeout(self, run, sleep):
        with self.assertRaises(subprocess.CalledProcessError):
            wait.wait_for_artifact("owner/repo", "123", "pages")
        sleep.assert_not_called()


if __name__ == "__main__":
    unittest.main()
