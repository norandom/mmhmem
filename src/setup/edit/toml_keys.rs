//! Key-only removal from one TOML table (reliable-installers task 3.4
//! follow-up): mmhmem takes out the keys it manages and leaves keys the user
//! added inside its table byte-identical.

use toml_edit::{DocumentMut, TableLike};

use super::{TomlEditError, checked, remove_table};

/// `input` without `keys` of the table at `path`. Every other key, comment
/// and blank line stays as it is. A table left without keys is removed
/// whole (with its sub-tables, see [`remove_table`]); an absent table gives
/// the input unchanged.
pub fn remove_keys(input: &str, path: &[&str], keys: &[&str]) -> Result<String, TomlEditError> {
    if path.is_empty() {
        return Err(TomlEditError::EmptyPath);
    }
    let crlf = input.contains("\r\n");
    if crlf && input.replace("\r\n", "").contains('\n') {
        return Err(TomlEditError::MixedLineEndings);
    }
    let lf = input.replace("\r\n", "\n");
    let mut doc: DocumentMut = lf.parse().map_err(|_| TomlEditError::InvalidOutput)?;
    let mut table: &mut dyn TableLike = doc.as_table_mut();
    for seg in path {
        let Some(item) = table.get_mut(seg) else {
            return Ok(input.to_owned());
        };
        table = item
            .as_table_like_mut()
            .ok_or(TomlEditError::UnsupportedLayout)?;
    }
    if table.iter().all(|(k, _)| keys.contains(&k)) {
        return remove_table(input, path);
    }
    for key in keys {
        table.remove(key);
    }
    let text = doc.to_string();
    checked(if crlf {
        text.replace('\n', "\r\n")
    } else {
        text
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: [&str; 2] = ["mcp_servers", "mmhmem"];
    const KEYS: [&str; 2] = ["command", "args"];

    #[test]
    fn only_managed_keys_go_and_user_keys_stay_byte_identical() {
        let input = "model = \"o3\" # mine\r\n\r\n[mcp_servers.mmhmem]\r\ncommand = \"/bin/mmhmem\"\r\nargs = [\"stdio\"]\r\ntool_timeout_sec = 60 # long\r\n\r\n[other]\r\nx = 1\r\n";
        let out = remove_keys(input, &PATH, &KEYS).unwrap();
        assert_eq!(
            out,
            "model = \"o3\" # mine\r\n\r\n[mcp_servers.mmhmem]\r\ntool_timeout_sec = 60 # long\r\n\r\n[other]\r\nx = 1\r\n"
        );
    }

    #[test]
    fn a_table_with_only_managed_keys_is_removed_whole() {
        let input = "a = 1\n\n[mcp_servers.mmhmem]\ncommand = \"m\"\nargs = [\"stdio\"]\n";
        assert_eq!(remove_keys(input, &PATH, &KEYS).unwrap(), "a = 1\n");
    }

    #[test]
    fn an_absent_table_leaves_the_input_unchanged() {
        let input = "a = 1\n";
        assert_eq!(remove_keys(input, &PATH, &KEYS).unwrap(), input);
    }

    #[test]
    fn a_sub_table_counts_as_a_user_key() {
        let input = "[mcp_servers.mmhmem]\ncommand = \"m\"\nargs = []\n\n[mcp_servers.mmhmem.env]\nK = \"v\"\n";
        let out = remove_keys(input, &PATH, &KEYS).unwrap();
        assert!(!out.contains("command"));
        assert!(out.contains("[mcp_servers.mmhmem.env]\nK = \"v\"\n"));
    }
}
