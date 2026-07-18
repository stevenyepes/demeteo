---
name: pr-review
description: Fetch open GitHub PRs for this repo and run /code-review against the one you pick, posting findings as inline PR comments. Use when the user asks to review MRs/PRs, "review the open PRs", or wants a PR reviewed end-to-end from listing to comments.
---

# PR Review

List open PRs, let the user pick one, check it out, then delegate the actual review to `/code-review --comment`. This skill only handles PR discovery and checkout — all review logic lives in `code-review`.

## Steps

1. **List open PRs** — run `scripts/list_prs.py`. It prints each open PR's number, title, author, draft/review status, and branch.
2. **Ask the user which PR to review** (one option per PR, by number). Stop here if there are no open PRs.
3. **Check out the PR branch** — run `scripts/checkout_pr.py <number>`. It refuses if the working tree is dirty (commit or stash first) and prints the branch you were on before, so you can return to it afterward.
4. **Run the review** — invoke the `code-review` skill with `--comment` so findings post as inline PR comments.
5. **Report** — summarize what was posted (or that no findings survived review), and remind the user which branch to switch back to (from step 3's output).

## Notes

- Requires `gh` authenticated with repo access (`gh auth status`).
- Never bypass `checkout_pr.py`'s dirty-tree check with `git checkout -f` — investigate the uncommitted work instead.
