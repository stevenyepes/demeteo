# Product Requirements Document (PRD): Demeteo

## 1. Executive Summary

Demeteo is a high-performance desktop orchestrator built on Tauri v2 (Rust/React). It serves as a secure Control Plane for integrating AI coding agents (Claude Code, OpenHands/OpenCode, Hermes) into live local and remote development environments. Rather than acting as a Virtual File System, Demeteo functions as a Policy Decision Point (PDP), acting as an intelligent proxy between autonomous agent intent and secure infrastructure execution via SSH.

## 2. Problem Statement

The integration of autonomous coding agents into professional workflows is currently dangerous and chaotic. Agents running directly in remote environments operate as "black boxes," risking destructive changes to live infrastructure, polluting Git repositories, and executing unverified code. Existing tools fail to provide a unified, human-in-the-loop (HITL) interface that can enforce strict permissions without slowing down the development cycle.

## 3. Core Architecture & Philosophy

Demeteo is built on three foundational pillars:

- **The Agent is the Executor; Demeteo is the Gatekeeper**: Demeteo does not manage file state directly. Agents issue structured tool calls (e.g., `edit_file`, `execute_bash`), and Demeteo intercepts these requests, surfacing them for human approval before returning the execution permission to the agent.
- **Stateless Sandboxing via Git Worktrees**: For complex feature development, Demeteo automatically isolates agent environments using Git Worktrees, ensuring the main branch remains pristine while natively supporting version control.
- **The Orchestrator, Not a Chatbot**: The UI prioritizes state management, telemetry, and actionable diffs over conversational chat, enforcing a "Mission Control" user experience.

## 4. Key Workflows & Features

### 4.1. Dual-Mode Execution Strategy

- **Project Mode (Git-Aware Sandbox)**: When assigned to a Git repository, Demeteo automatically provisions a new Git Worktree and branch (e.g., `feature/agent-draft`) on the target machine. The agent is bound exclusively to this worktree.
- **Ad-Hoc Mode (Raw Session)**: For lightweight tasks (e.g., log analysis, rapid scripting), the agent is spawned directly in the active directory without Git overhead, relying entirely on the Permission Proxy for safety.

### 4.2. The Permission Proxy & Approval Queue

- **Automatic 'Read' Approvals**: Non-destructive tool calls (e.g., `ls`, `cat`, `grep`) are automatically approved by Demeteo and executed in the background to prevent alert fatigue.
- **Strict 'Write/Execute' Intercepts**: Any command that mutates state (e.g., file edits, `cargo build`, `rm`) is intercepted. The UI pauses the agent thread and renders a rich Diff/Command block.
- **Human-in-the-Loop Override**: The user can Approve, Reject, or Modify the intent. If rejected with feedback, the feedback is passed back to the agent's context window for recalculation.

### 4.3. User Interface (The Supervisor Dashboard)

- **Active Threads (Agent Workspaces)**: Contextually isolated sessions that map to specific Worktrees or Ad-Hoc sessions, allowing seamless multitasking between agents.
- **Contextual Split-Pane (Code Inspector)**: A slide-out, read-only Monaco-style editor that allows the developer to deeply inspect file context surrounding an agent's proposed diff without leaving the orchestrator view.
- **Workspace Toggle**: Instant flipping between the AI "Supervisor Stream" and a raw, interactive "Terminal" SSH session.
- **Environment Manager**: UI to configure local, staging, and production SSH targets, complete with granular toggles for enabling specific agent integrations per environment.

## 5. Technical Stack

- **Frontend**: React, TypeScript, Vite, TailwindCSS.
- **Backend**: Rust (Tauri v2), russh (for SSH tunneling), tokio (for async WebSocket/IPC streams).
- **Agent Adapters (Phase 3)**: Standardized Rust traits to normalize outputs from MCP (Anthropic), WebSockets (OpenHands), and local inference REST APIs (Ollama/vLLM) into a unified `DemeteoEvent` struct.

## 6. Success Metrics

- **Zero Main-Branch Pollution**: 100% of Project Mode agent edits must be contained within their respective Git Worktrees.
- **Intercept Latency**: The round-trip from an agent's write intent to the UI rendering the Approval Block must be < 50ms.
- **Session Resiliency**: Background threads must successfully resume state even if the SSH tunnel temporarily drops.

---

## What's Next

This PRD solidifies Demeteo not just as a UI, but as a robust, professional-grade infrastructure tool.

When you are ready to begin implementation, we can start mapping out the specific Rust structs for the Permission Proxy event loop!
