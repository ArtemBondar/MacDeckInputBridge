"""Wait until GitHub's Actions API lists this attempt's Pages artifact."""

import json
import os
import subprocess
import sys
import time

# GitHub's artifact index was not immediately consistent after a successful
# upload. Allow up to one minute for indexing, polling every two seconds.
VISIBILITY_TIMEOUT_SECONDS = 60
POLL_INTERVAL_SECONDS = 2
MAX_CHECKS = VISIBILITY_TIMEOUT_SECONDS // POLL_INTERVAL_SECONDS + 1


def wait_for_artifact(repo, run_id, artifact_name):
    endpoint = f"repos/{repo}/actions/runs/{run_id}/artifacts?per_page=100"
    for check in range(1, MAX_CHECKS + 1):
        result = subprocess.run(
            ["gh", "api", endpoint, "--paginate", "--slurp"],
            check=True, capture_output=True, text=True,
        )
        pages = json.loads(result.stdout)
        matches = [
            artifact for page in pages for artifact in page["artifacts"]
            if artifact["name"] == artifact_name and not artifact["expired"]
        ]
        if len(matches) == 1:
            return True
        if len(matches) > 1:
            raise ValueError(f"Multiple artifacts named {artifact_name} found")
        if check < MAX_CHECKS:
            print(f"Waiting for {artifact_name} ({check}/{MAX_CHECKS})", flush=True)
            time.sleep(POLL_INTERVAL_SECONDS)
    return False


def main():
    try:
        if wait_for_artifact(os.environ["GITHUB_REPOSITORY"], os.environ["GITHUB_RUN_ID"], os.environ["PAGES_ARTIFACT"]):
            return 0
        print("::error::Pages artifact did not become available", file=sys.stderr)
    except (KeyError, ValueError, subprocess.CalledProcessError) as error:
        print(f"::error::Pages artifact lookup failed: {error}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
