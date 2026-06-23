use crate::domain::ids::MachineId;
use crate::domain::models::Machine;
use crate::ports::db::MachineRepository;

/// Resolve machine by machine_id string. Supports matching by MachineId (UUID),
/// format username@host, host, or name.
pub fn resolve_machine(
    machines: &dyn MachineRepository,
    machine_id: &str,
) -> Result<Machine, String> {
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
