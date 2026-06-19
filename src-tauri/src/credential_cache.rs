use std::collections::HashMap;
use std::sync::Mutex;

static CACHE: std::sync::LazyLock<Mutex<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Retrieve a cached credential, or fetch + cache it.
pub fn get_or_fetch(key: &str, fetcher: impl FnOnce() -> Result<String, String>) -> Result<String, String> {
    let mut cache = CACHE.lock().map_err(|e| format!("Cache lock error: {}", e))?;
    if let Some(value) = cache.get(key) {
        return Ok(value.clone());
    }
    let value = fetcher()?;
    cache.insert(key.to_string(), value.clone());
    Ok(value)
}

/// Invalidate a cached credential (e.g. after write or delete).
pub fn invalidate(key: &str) {
    if let Ok(mut cache) = CACHE.lock() {
        cache.remove(key);
    }
}

/// Clear all cached credentials.
#[allow(dead_code)]
pub fn clear() {
    if let Ok(mut cache) = CACHE.lock() {
        cache.clear();
    }
}
