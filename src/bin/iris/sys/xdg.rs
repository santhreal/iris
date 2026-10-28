//! Desktop entries (the Desktop Entry Specification): the autostart
//! entry Start at login writes and the application menu entry an
//! AppImage writes. One quoting of `Exec` and string values, and one
//! write a desktop reads whole or not at all.

use std::path::Path;

/// `program` as one quoted `Exec` argument (Desktop Entry
/// Specification, "The Exec key"). In a quoted argument `"`, `` ` ``,
/// and `$` take a backslash and a backslash takes another; the string
/// escape rule then doubles every backslash. `%` doubles.
pub fn exec_arg(program: &Path) -> Result<String, String> {
    let text = utf8(program)?;
    let mut arg = String::with_capacity(text.len() + 2);
    arg.push('"');
    for c in text.chars() {
        match c {
            '"' | '`' | '$' => {
                arg.push_str(r"\\");
                arg.push(c);
            }
            '\\' => arg.push_str(r"\\\\"),
            '%' => arg.push_str("%%"),
            c if c.is_control() => return Err(control(program)),
            c => arg.push(c),
        }
    }
    arg.push('"');
    Ok(arg)
}

/// `path` as a string value, such as `TryExec` or `Icon`: a backslash
/// doubles.
pub fn string_value(path: &Path) -> Result<String, String> {
    let text = utf8(path)?;
    if text.chars().any(char::is_control) {
        return Err(control(path));
    }
    Ok(text.replace('\\', r"\\"))
}

fn utf8(path: &Path) -> Result<&str, String> {
    path.to_str().ok_or_else(|| {
        format!(
            "{} is not UTF-8, which a desktop entry holds",
            path.display()
        )
    })
}

fn control(path: &Path) -> String {
    format!(
        "{} holds a control character, which a desktop entry cannot",
        path.display()
    )
}

/// Write `bytes` to `path` through a sibling a desktop does not read
/// (no .desktop suffix) and one rename, so a desktop that reads the
/// file meanwhile reads the old one or the new one.
pub fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let staged = dir.join(format!(".{name}.new"));
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&staged, bytes))
        .and_then(|()| std::fs::rename(&staged, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            format!("write {}: {e}", path.display())
        })
}
