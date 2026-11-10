pub mod report;

pub use report::{parse, ReportRow};

/// Who sent a report. Sources are identified by the sync client; names and projects are
/// private and never leave the server.
#[derive(Debug, Clone)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub agent: String,
    pub project: String,
}

pub fn valid_identifier(value: &str, max_length: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_length
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._:@+-".contains(&byte))
}

/// Strips control characters and caps length on a char boundary.
pub fn clean_label(value: &str, max_length: usize) -> String {
    let mut cleaned: String = value.chars().filter(|c| !c.is_control()).collect();
    if cleaned.len() > max_length {
        let mut end = max_length;
        while !cleaned.is_char_boundary(end) {
            end -= 1;
        }
        cleaned.truncate(end);
    }
    cleaned.trim().to_owned()
}
