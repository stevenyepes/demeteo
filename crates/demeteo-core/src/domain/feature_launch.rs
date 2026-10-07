//! The cheap argument refusals every Feature launch shares, so a caller that
//! does expensive work before launching (staging attachments) can refuse the
//! same blank input with the same words, ahead of that work.

const BLANK_TITLE: &str = "Feature title cannot be empty.";
const BLANK_DESCRIPTION: &str = "Feature description cannot be empty.";

pub fn refuse_blank_launch_text(title: &str, description: &str) -> Result<(), String> {
    if title.trim().is_empty() {
        return Err(BLANK_TITLE.to_string());
    }
    if description.trim().is_empty() {
        return Err(BLANK_DESCRIPTION.to_string());
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/domain/feature_launch.rs"]
mod tests;
