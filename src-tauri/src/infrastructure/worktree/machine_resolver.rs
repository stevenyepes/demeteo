use crate::domain::ids::MachineId;
use crate::domain::models::Machine;
use crate::ports::db::MachineRepository;

pub fn local_machine() -> Machine {
    Machine {
        id: MachineId::from("local"),
        name: "local".to_string(),
        host: "localhost".to_string(),
        port: 22,
        username: String::new(),
        auth_type: "local".to_string(),
        key_path: None,
        agents: None,
        auto_approved_rules: None,
        use_login_shell: None,
        setup_commands: None,
    }
}

/// Resolve machine by machine_id string. Supports matching by MachineId (UUID),
/// format username@host, host, or name.
pub fn resolve_machine(
    machines: &dyn MachineRepository,
    machine_id: &str,
) -> Result<Machine, String> {
    // "local" is the built-in sentinel for the host machine — not stored in the DB.
    if machine_id.is_empty() || machine_id == "local" {
        return Ok(local_machine());
    }

    let machine_id_typed = MachineId::from(machine_id.to_string());

    // First try direct lookup by ID (fast path)
    if let Ok(Some(m)) = machines.get_machine(&machine_id_typed) {
        return Ok(m);
    }

    // Fallback: search the list of machines
    let list = machines.get_machines()?;
    list.into_iter()
        .find(|m| {
            m.id == machine_id_typed
                || format!("{}@{}", m.username, m.host) == machine_id
                || m.host == machine_id
                || m.name == machine_id
        })
        .ok_or_else(|| format!("Machine not found: {}", machine_id))
}
