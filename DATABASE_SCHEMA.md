# Database Schema Spec: demeteo.db

This document outlines the SQLite schema used by the Demeteo desktop orchestrator. The database file resides in the user’s local app data directory (e.g., `~/.local/share/com.demeteo.app/` or `%APPDATA%\com.demeteo.app\`). 

Foreign key constraints are enabled globally to preserve referential integrity.

---

## 🗄️ Tables Configuration

### 1. `machines`
Stores target connection profiles for remote servers and local nodes.

```sql
CREATE TABLE machines (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL,
    username TEXT NOT NULL,
    auth_type TEXT NOT NULL, -- 'key', 'password', 'agent'
    key_path TEXT,
    agents TEXT,             -- JSON array of active agents e.g. ["Claude Code"]
    auto_approved_rules TEXT, -- JSON array of regex command patterns
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

### 2. `agent_profiles`
Stores configuration profiles for AI agents running on specific target machines.

```sql
CREATE TABLE agent_profiles (
    id TEXT PRIMARY KEY,
    machine_id TEXT NOT NULL,
    name TEXT NOT NULL,
    agent_type TEXT NOT NULL, -- 'ollama', 'openai', 'cli', 'custom_http'
    command TEXT,             -- Optional startup shell command
    work_dir TEXT,            -- Working directory on host
    port INTEGER,             -- API port to forward
    ready_check TEXT,         -- e.g. 'GET /health' or log string check
    FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
);
```

### 3. `chat_sessions`
Organizes conversations conducted with AI agents.

```sql
CREATE TABLE chat_sessions (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    title TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(agent_id) REFERENCES agent_profiles(id) ON DELETE CASCADE
);
```

### 4. `chat_messages`
Stores message content and timestamps within active chats.

```sql
CREATE TABLE chat_messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    sender TEXT NOT NULL, -- 'user', 'agent'
    content TEXT NOT NULL,
    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE
);
```

### 5. `session_history`
Maintains raw records of connection events and terminal output buffer histories.

```sql
CREATE TABLE session_history (
    id TEXT PRIMARY KEY,
    machine_id TEXT NOT NULL,
    session_type TEXT NOT NULL, -- 'terminal', 'agent'
    title TEXT NOT NULL,
    content TEXT,               -- Buffer or logs dump
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
);
```
