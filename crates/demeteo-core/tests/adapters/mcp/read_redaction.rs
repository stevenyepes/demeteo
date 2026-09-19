// Tests for `bound_error_messages` in `src/adapters/mcp/mcp_handler.rs`
// (mirrored-tests convention). `super` = `adapters::mcp::mcp_handler`.

use super::*;

#[test]
fn error_messages_are_scrubbed_and_bounded_wherever_they_sit_in_a_result() {
    let secret = "ghp_0123456789abcdefABCDEF0123456789abcdef";
    let long_tail = format!("{}\nfinal line", "filler line\n".repeat(2_000));
    let mut value = json!({
        "id": "f-1",
        "error_message": format!("clone failed: https://u:{secret}@github.com/o/r.git"),
        "steps": [
            { "error_message": long_tail, "note": secret },
            { "error_message": null },
        ],
    });

    bound_error_messages(&mut value);

    let first = value["error_message"].as_str().unwrap();
    assert!(!first.contains(secret), "{first}");
    let step = value["steps"][0]["error_message"].as_str().unwrap();
    assert!(step.len() <= ERROR_MESSAGE_BUDGET_BYTES);
    assert!(step.ends_with("final line"), "the tail is what survives");
    assert_eq!(
        value["steps"][0]["note"], secret,
        "only `error_message` fields are touched"
    );
    assert!(value["steps"][1]["error_message"].is_null());
}
