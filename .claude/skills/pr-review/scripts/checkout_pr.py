#!/usr/bin/env python3
"""Safely check out a PR branch for review: refuses if the working tree is dirty."""
import argparse
import subprocess
import sys


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("pr_number", type=int)
    args = parser.parse_args()

    status = run(["git", "status", "--porcelain"])
    if status.stdout.strip():
        sys.exit(
            "Working tree has uncommitted changes. Commit or stash them "
            "before checking out the PR branch."
        )

    current = run(["git", "branch", "--show-current"]).stdout.strip()

    result = run(["gh", "pr", "checkout", str(args.pr_number)])
    if result.returncode != 0:
        sys.exit(f"gh pr checkout failed: {result.stderr.strip()}")

    print(f"Checked out PR #{args.pr_number}. Previous branch: {current}")


if __name__ == "__main__":
    main()
