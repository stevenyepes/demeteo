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
