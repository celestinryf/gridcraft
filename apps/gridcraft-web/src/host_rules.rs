//! The rules for hosted mode (see `host.rs`) that don't need a browser, so they are tested
//! natively.

/// Longest file name kept from the host.
pub const MAX_NAME_CHARS: usize = 255;
/// Longest document token kept from the host.
pub const MAX_DOC_CHARS: usize = 100;
/// Longest user name taken from `?author=`.
pub const MAX_AUTHOR_CHARS: usize = 200;
/// Largest workbook the host may hand in, in bytes (as Open allows for a picked file).
pub const MAX_OPEN_BYTES: u32 = 200 << 20;
/// The commands a host may run. Saving is how it collects the workbook; undo and redo let its own
/// shortcuts reach the workbook.
pub const HOST_COMMANDS: &[&str] = &["file.save", "edit.undo", "edit.redo"];
/// Saving a file with one of these extensions hands the whole workbook to the host: the workbook
/// counts as saved under that name, as it would after Save on the desktop. Other files (CSV,
/// PDF…) are exports, which leave the unsaved changes as they are.
pub const WORKBOOK_EXTS: &[&str] = &["xlsx"];

/// The last part of a host-given name, trimmed to a sane length; `None` for an empty one.
pub fn clean_name(raw: &str) -> Option<String> {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("").trim();
    if last.is_empty() {
        return None;
    }
    Some(last.chars().take(MAX_NAME_CHARS).collect())
}

/// A host-given document token, cut to its longest allowed length.
pub fn clean_doc(raw: &str) -> String {
    raw.chars().take(MAX_DOC_CHARS).collect()
}

/// The user's name from `?author=`; `None` when it is empty.
pub fn clean_author(raw: &str) -> Option<String> {
    let author: String = raw.trim().chars().take(MAX_AUTHOR_CHARS).collect();
    (!author.is_empty()).then_some(author)
}

/// Whether a host may run this command.
pub fn host_may_run(id: &str) -> bool {
    HOST_COMMANDS.contains(&id)
}

/// Whether saving a file with this name saves the workbook itself (see [`WORKBOOK_EXTS`]).
pub fn is_workbook(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| WORKBOOK_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_keep_their_last_part_and_a_sane_length() {
        assert_eq!(clean_name(r"C:\notes\Budget.xlsx").as_deref(), Some("Budget.xlsx"));
        assert_eq!(clean_name("a/b/Plan.ods").as_deref(), Some("Plan.ods"));
        assert_eq!(clean_name("  spaced.xlsx ").as_deref(), Some("spaced.xlsx"));
        assert_eq!(clean_name("mixed/path\\Name.csv").as_deref(), Some("Name.csv"));
        assert_eq!(clean_name(""), None);
        assert_eq!(clean_name("   "), None);
        assert_eq!(clean_name("folder/"), None);
        assert_eq!(clean_name("folder\\"), None);
        assert_eq!(clean_name(&"é".repeat(400)).map(|n| n.chars().count()), Some(MAX_NAME_CHARS));
        assert_eq!(clean_name(&format!("dir/{}", "x".repeat(MAX_NAME_CHARS))).map(|n| n.len()), Some(MAX_NAME_CHARS));
    }

    #[test]
    fn a_host_may_only_save_undo_and_redo() {
        for id in ["file.save", "edit.undo", "edit.redo"] {
            assert!(host_may_run(id), "{id}");
        }
        for id in ["file.close", "file.open", "file.new", "file.saveAs", "cell.set", "File.save", "file.save ", ""] {
            assert!(!host_may_run(id), "{id:?}");
        }
    }

    #[test]
    fn host_commands_are_commands_of_the_engine() {
        let ids: Vec<&str> = gridcraft_engine::Session::new().commands().into_iter().map(|c| c.id).collect();
        for id in HOST_COMMANDS {
            assert!(ids.contains(id), "{id} is a command");
        }
    }

    #[test]
    fn document_tokens_are_cut_to_length() {
        assert_eq!(clean_doc("abc"), "abc");
        assert_eq!(clean_doc(""), "");
        assert_eq!(clean_doc(&"é".repeat(150)).chars().count(), MAX_DOC_CHARS);
    }

    #[test]
    fn the_author_is_trimmed_cut_and_never_empty() {
        assert_eq!(clean_author("  Ada  ").as_deref(), Some("Ada"));
        assert_eq!(clean_author(""), None);
        assert_eq!(clean_author(" \t "), None);
        assert_eq!(clean_author(&"a".repeat(300)).map(|a| a.len()), Some(MAX_AUTHOR_CHARS));
    }

    #[test]
    fn only_workbook_files_count_as_saving_the_workbook() {
        assert!(is_workbook("Budget.xlsx"));
        assert!(is_workbook("BUDGET.XLSX"));
        assert!(is_workbook("a.b.xlsx"));
        for name in ["Budget.csv", "Budget.pdf", "Budget.xlsm", "xlsx", "Budget.xlsx.bak", ""] {
            assert!(!is_workbook(name), "{name:?}");
        }
    }
}
