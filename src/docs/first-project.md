# Your First Project

Demeteo orchestrates multi-agent workflows against Git repositories. This guide walks you through creating your first project from scratch.

## Prerequisites

- A GitHub or GitLab account with a repository you want to work on
- A Personal Access Token (PAT) with `repo` scope (GitHub) or `api` scope (GitLab)

## Step 1: Connect a Provider

Click **Providers** in the top bar, then **Connect Provider**. Fill in:

- **Host**: `github.com` or `gitlab.com` (or your self-hosted instance)
- **PAT**: Your personal access token
- **Username**: Your Git username

Once connected, Demeteo can discover your repositories.

## Step 2: Create a Project

From the empty state, click **Sync Worktrees** or use the **+** button in the sidebar. Select:

1. A **name** for your project (e.g. "My API Service")
2. The **compute target** — local or remote SSH machine
3. One or more **repositories** to clone

Click **Save** to begin bootstrapping. Demeteo will clone your repos and analyze the codebase to propose a worktree strategy.

## Step 3: Approve the Strategy

After cloning, Demeteo shows a **strategy proposal** with:

- Default branch (e.g. `main`, `master`)
- Branch prefix for feature branches
- Optional test command
- Conflict resolution policy

Review and adjust these settings, then click **Approve & Build Workspace**.

## Step 4: Run a Feature

Your project home screen shows a feature input. Describe what you want built:

> "Add input validation to the login endpoint"

Demeteo infers the right workflow, lets you customize agent and model settings, and starts the pipeline. Each step runs in sequence — research, spec, plan, implement, validate — with gates where human approval is needed.
