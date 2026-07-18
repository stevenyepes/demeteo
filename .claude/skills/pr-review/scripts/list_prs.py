#!/usr/bin/env python3
"""List open PRs on this repo's GitHub remote, formatted for picking one to review."""
import json
import subprocess
import sys

FIELDS = "number,title,author,headRefName,isDraft,reviewDecision,updatedAt"


def main():
    try:
        raw = subprocess.run(
            ["gh", "pr", "list", "--state", "open", "--json", FIELDS],
            capture_output=True, text=True, check=True,
        )
    except FileNotFoundError:
        sys.exit("gh CLI not found. Install it: https://cli.github.com")
    except subprocess.CalledProcessError as e:
        sys.exit(f"gh pr list failed: {e.stderr.strip()}")

    prs = json.loads(raw.stdout)
    if not prs:
        print("No open PRs.")
        return

    prs.sort(key=lambda p: p["updatedAt"], reverse=True)
    for pr in prs:
        flags = ["draft"] if pr["isDraft"] else []
        flags.append((pr["reviewDecision"] or "no review").lower().replace("_", " "))
        print(
            f"#{pr['number']:<5} {pr['title']:<60.60} "
            f"{pr['author']['login']:<15} [{', '.join(flags)}] "
            f"({pr['headRefName']})"
        )


if __name__ == "__main__":
    main()
