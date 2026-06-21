use crate::domain::ids::MachineId;
use crate::domain::models::Machine;

/// Tests SSH connectivity using parameters passed directly from the UI form.
/// This avoids stale-state bugs where the DB has outdated auth settings that the
/// user has already changed in the form but not yet saved.
#[tauri::command]
pub fn test_ssh_connection(
    host: String,
    port: i32,
    username: String,
    auth_type: String,
    key_path: Option<String>,
    secret: Option<String>,
) -> Result<(), String> {
    if auth_type == "local" {
        return Ok(());
    }

    let machine = Machine {
        id: MachineId::from(String::new()),
        name: String::new(),
        host,
        port,
        username,
        auth_type,
        key_path,
        agents: None,
        auto_approved_rules: None,
        use_login_shell: None,
        setup_commands: None,
    };

    let (sess, _tcp) = crate::ssh_util::connect(&machine, secret)?;
    let _ = sess.disconnect(None, "test complete", None);
    Ok(())
}
