use rusqlite::params;

use crate::domain::ids::ProviderId;
use crate::domain::models::ProviderInstance;
use crate::ports::db::AppSettingsRepository;

use super::super::SqliteAdapter;

impl AppSettingsRepository for SqliteAdapter {
    fn add_provider_instance(&self, provider: ProviderInstance) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT OR REPLACE INTO provider_instances (id, kind, host, username, avatar_url, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                provider.id, provider.kind, provider.host,
                provider.username, provider.avatar_url, provider.created_at
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_provider_instances(&self) -> Result<Vec<ProviderInstance>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, host, username, avatar_url, created_at
                 FROM provider_instances ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map([], |row| {
                Ok(ProviderInstance {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    host: row.get(2)?,
                    username: row.get(3)?,
                    avatar_url: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn delete_provider_instance(&self, id: &ProviderId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "DELETE FROM provider_instances WHERE id = ?1",
            params![id.0],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_app_session(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock()?;
        Ok(conn
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                params![key],
                |r| r.get(0),
            )
            .ok())
    }

    fn set_app_session(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO app_settings (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete_app_session(&self, key: &str) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute("DELETE FROM app_settings WHERE key=?1", params![key])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn app_setting_get(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock()?;
        Ok(conn
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                params![key],
                |r| r.get(0),
            )
            .ok())
    }

    fn app_setting_set(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO app_settings (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}
