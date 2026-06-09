use rusqlite::{params, Connection, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct DbState {
    pub conn: Mutex<Connection>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Machine {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i32,
    pub username: String,
    pub auth_type: String, // 'key', 'password', 'agent'
    pub key_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AgentProfile {
    pub id: String,
    pub machine_id: String,
    pub name: String,
    pub agent_type: String, // 'ollama', 'openai', 'cli', 'custom_http'
    pub command: Option<String>,
    pub work_dir: Option<String>,
    pub port: Option<i32>,
    pub ready_check: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatSession {
    pub id: String,
    pub agent_id: String,
    pub title: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub id: String,
    pub session_id: String,
    pub sender: String, // 'user', 'agent'
    pub content: String,
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionHistory {
    pub id: String,
    pub machine_id: String,
    pub session_type: String, // 'terminal', 'agent'
    pub title: String,
    pub content: Option<String>,
    pub created_at: String,
}

/// Initializes the SQLite database file in the app data directory and creates tables.
pub fn init_db(app_data_dir: PathBuf) -> Result<Connection> {
    if !app_data_dir.exists() {
        fs::create_dir_all(&app_data_dir).expect("Failed to create app data directory");
    }

    let db_path = app_data_dir.join("demeteo.db");
    let conn = Connection::open(db_path)?;

    conn.execute("PRAGMA foreign_keys = ON;", [])?;

    conn.execute_batch(
        "BEGIN;
        
        CREATE TABLE IF NOT EXISTS machines (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            host TEXT NOT NULL,
            port INTEGER NOT NULL,
            username TEXT NOT NULL,
            auth_type TEXT NOT NULL,
            key_path TEXT,
            agents TEXT,
            auto_approved_rules TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS agent_profiles (
            id TEXT PRIMARY KEY,
            machine_id TEXT NOT NULL,
            name TEXT NOT NULL,
            agent_type TEXT NOT NULL,
            command TEXT,
            work_dir TEXT,
            port INTEGER,
            ready_check TEXT,
            FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS chat_sessions (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            title TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(agent_id) REFERENCES agent_profiles(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS chat_messages (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            sender TEXT NOT NULL,
            content TEXT NOT NULL,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(session_id) REFERENCES chat_sessions(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS session_history (
            id TEXT PRIMARY KEY,
            machine_id TEXT NOT NULL,
            session_type TEXT NOT NULL,
            title TEXT NOT NULL,
            content TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS thread_sessions (
            id TEXT PRIMARY KEY,
            machine_id TEXT NOT NULL,
            title TEXT NOT NULL,
            mode TEXT NOT NULL,
            branch TEXT,
            repo_path TEXT,
            sandbox_path TEXT,
            status TEXT NOT NULL,
            agent_kind TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(machine_id) REFERENCES machines(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS thread_working_memory (
            thread_id      TEXT NOT NULL,
            file_path      TEXT NOT NULL,
            line_count     INTEGER,
            size_bytes     INTEGER,
            modified_at    INTEGER,
            first_read_at  INTEGER NOT NULL,
            last_read_at   INTEGER NOT NULL,
            PRIMARY KEY (thread_id, file_path),
            FOREIGN KEY (thread_id) REFERENCES thread_sessions(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_twm_thread_last_read
            ON thread_working_memory(thread_id, last_read_at DESC);

        COMMIT;"
    )?;

    Ok(conn)
}

/* ----------------------------------------------------
   DATABASE CRUD FUNCTIONS
   ---------------------------------------------------- */

pub fn add_machine(conn: &Connection, m: Machine) -> Result<()> {
    conn.execute(
        "INSERT INTO machines (id, name, host, port, username, auth_type, key_path)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![m.id, m.name, m.host, m.port, m.username, m.auth_type, m.key_path],
    )?;
    Ok(())
}

pub fn get_machines(conn: &Connection) -> Result<Vec<Machine>> {
    let mut stmt = conn.prepare("SELECT id, name, host, port, username, auth_type, key_path FROM machines ORDER BY created_at DESC")?;
    let machine_iter = stmt.query_map([], |row| {
        Ok(Machine {
            id: row.get(0)?,
            name: row.get(1)?,
            host: row.get(2)?,
            port: row.get(3)?,
            username: row.get(4)?,
            auth_type: row.get(5)?,
            key_path: row.get(6)?,
        })
    })?;

    let mut list = Vec::new();
    for machine in machine_iter {
        list.push(machine?);
    }
    Ok(list)
}

pub fn delete_machine(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM machines WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn update_machine(conn: &Connection, m: Machine) -> Result<()> {
    conn.execute(
        "UPDATE machines 
         SET name = ?2, host = ?3, port = ?4, username = ?5, auth_type = ?6, key_path = ?7 
         WHERE id = ?1",
        params![m.id, m.name, m.host, m.port, m.username, m.auth_type, m.key_path],
    )?;
    Ok(())
}

pub fn add_agent_profile(conn: &Connection, a: AgentProfile) -> Result<()> {
    conn.execute(
        "INSERT INTO agent_profiles (id, machine_id, name, agent_type, command, work_dir, port, ready_check)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![a.id, a.machine_id, a.name, a.agent_type, a.command, a.work_dir, a.port, a.ready_check],
    )?;
    Ok(())
}

pub fn get_agent_profiles(conn: &Connection, machine_id: &str) -> Result<Vec<AgentProfile>> {
    let mut stmt = conn.prepare("SELECT id, machine_id, name, agent_type, command, work_dir, port, ready_check FROM agent_profiles WHERE machine_id = ?1")?;
    let agent_iter = stmt.query_map(params![machine_id], |row| {
        Ok(AgentProfile {
            id: row.get(0)?,
            machine_id: row.get(1)?,
            name: row.get(2)?,
            agent_type: row.get(3)?,
            command: row.get(4)?,
            work_dir: row.get(5)?,
            port: row.get(6)?,
            ready_check: row.get(7)?,
        })
    })?;

    let mut list = Vec::new();
    for agent in agent_iter {
        list.push(agent?);
    }
    Ok(list)
}

pub fn delete_agent_profile(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM agent_profiles WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn create_chat_session(conn: &Connection, id: &str, agent_id: &str, title: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO chat_sessions (id, agent_id, title) VALUES (?1, ?2, ?3)",
        params![id, agent_id, title],
    )?;
    Ok(())
}

pub fn get_chat_sessions(conn: &Connection, agent_id: &str) -> Result<Vec<ChatSession>> {
    let mut stmt = conn.prepare("SELECT id, agent_id, title, created_at FROM chat_sessions WHERE agent_id = ?1 ORDER BY created_at DESC")?;
    let session_iter = stmt.query_map(params![agent_id], |row| {
        Ok(ChatSession {
            id: row.get(0)?,
            agent_id: row.get(1)?,
            title: row.get(2)?,
            created_at: row.get(3)?,
        })
    })?;

    let mut list = Vec::new();
    for s in session_iter {
        list.push(s?);
    }
    Ok(list)
}

pub fn add_chat_message(conn: &Connection, id: &str, session_id: &str, sender: &str, content: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO chat_messages (id, session_id, sender, content) VALUES (?1, ?2, ?3, ?4)",
        params![id, session_id, sender, content],
    )?;
    Ok(())
}

pub fn get_chat_messages(conn: &Connection, session_id: &str) -> Result<Vec<ChatMessage>> {
    let mut stmt = conn.prepare("SELECT id, session_id, sender, content, timestamp FROM chat_messages WHERE session_id = ?1 ORDER BY timestamp ASC")?;
    let msg_iter = stmt.query_map(params![session_id], |row| {
        Ok(ChatMessage {
            id: row.get(0)?,
            session_id: row.get(1)?,
            sender: row.get(2)?,
            content: row.get(3)?,
            timestamp: row.get(4)?,
        })
    })?;

    let mut list = Vec::new();
    for m in msg_iter {
        list.push(m?);
    }
    Ok(list)
}

pub fn add_session_history(conn: &Connection, id: &str, machine_id: &str, session_type: &str, title: &str, content: Option<&str>) -> Result<()> {
    conn.execute(
        "INSERT INTO session_history (id, machine_id, session_type, title, content) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, machine_id, session_type, title, content],
    )?;
    Ok(())
}

pub fn get_session_history(conn: &Connection, machine_id: &str) -> Result<Vec<SessionHistory>> {
    let mut stmt = conn.prepare("SELECT id, machine_id, session_type, title, content, created_at FROM session_history WHERE machine_id = ?1 ORDER BY created_at DESC")?;
    let history_iter = stmt.query_map(params![machine_id], |row| {
        Ok(SessionHistory {
            id: row.get(0)?,
            machine_id: row.get(1)?,
            session_type: row.get(2)?,
            title: row.get(3)?,
            content: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;

    let mut list = Vec::new();
    for h in history_iter {
        list.push(h?);
    }
    Ok(list)
}
