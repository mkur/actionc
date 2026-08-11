use std::fs;
use std::path::Path;

pub fn read_snapshot(path: &Path) -> String {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    normalize_newlines(text)
}

fn normalize_newlines(text: String) -> String {
    text.replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_windows_snapshot_line_endings() {
        assert_eq!(
            normalize_newlines("first\r\nsecond\r\n".to_string()),
            "first\nsecond\n"
        );
    }
}
