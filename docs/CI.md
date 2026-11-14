# CI checks

CI compares the pull request's merge commit with its base, or a push with its
previous commit. Known Mac-only and Deck-only paths select their respective
suite. Protocol, shared logic, dependencies, CI, releases, and unknown paths
run both. Renames include both names. Missing history or an empty comparison
runs both suites. The release workflow always runs its full checks.

The workflow always runs the `scope` and `CI result` checks. Selected platform
jobs must succeed for `CI result` to pass; unselected platform jobs are marked
skipped. Require `CI result` in branch protection so routing failures cannot
pass unnoticed. If existing `swift` and `receiver` checks are required, verify
the repository's protection rules accept GitHub's skipped-job success status
before changing requirements. This change does not modify branch protection.

To test the routing rules locally, run:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts -p 'test_ci_scope.py'
```
