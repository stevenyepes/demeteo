use super::*;

#[test]
fn newtype_is_transparent_for_serde() {
    let id: MachineId = "m-42".into();
    let json = serde_json::to_string(&id).unwrap();
    assert_eq!(json, "\"m-42\"");
    let back: MachineId = serde_json::from_str(&json).unwrap();
    assert_eq!(back, id);
}

#[test]
fn newtype_derefs_to_str() {
    let id = MachineId::new("m-1");
    let s: &str = &id;
    assert_eq!(s, "m-1");
}

#[test]
fn newtype_from_string_and_str() {
    let a: MachineId = String::from("x").into();
    let b: MachineId = "y".into();
    assert_eq!(a.as_str(), "x");
    assert_eq!(b.as_str(), "y");
}

/// Both spellings of "this host". The empty id is the one that gets missed:
/// it reaches port methods from callers that never had a machine to name, and
/// a check that only tests `"local"` silently sends it down the remote path.
#[test]
fn both_spellings_of_the_local_machine_are_local() {
    assert!(MachineId::from(LOCAL_MACHINE).is_local());
    assert!(MachineId::from(String::new()).is_local());
    assert!(!MachineId::from("m-42").is_local());
    assert!(
        !MachineId::from("localhost").is_local(),
        "the sentinel is exact — a machine may legitimately be named localhost"
    );
}
