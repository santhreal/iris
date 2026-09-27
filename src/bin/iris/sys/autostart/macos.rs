//! macOS: a LaunchAgent in `~/Library/LaunchAgents`, which launchd
//! loads at login and, with `RunAtLoad`, starts once.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// `~/Library/LaunchAgents/dev.iris.app.plist`.
fn agent() -> Result<PathBuf, String> {
    let base = directories::BaseDirs::new()
        .ok_or_else(|| "start at login: this account has no home directory".to_string())?;
    Ok(base
        .home_dir()
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", iris_lib::APP_ID)))
}

fn this_iris() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("start at login: find this iris: {e}"))
}

/// Whether a LaunchAgent starts this iris at login.
pub fn enabled() -> bool {
    match (agent(), this_iris()) {
        (Ok(agent), Ok(program)) => enabled_at(&agent, &program),
        _ => false,
    }
}

/// Make this iris start at login, or start nothing.
pub fn set(on: bool) -> Result<(), String> {
    set_at(&agent()?, &this_iris()?, on)
}

/// Whether the agent at `agent` runs `program`. A hand-installed copy
/// of packaging/macos/dev.iris.app.plist counts when it names this
/// program.
fn enabled_at(agent: &Path, program: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(agent) else {
        return false;
    };
    text.contains(&format!(
        "<string>{}</string>",
        xml(&program.to_string_lossy())
    ))
}

fn set_at(agent: &Path, program: &Path, on: bool) -> Result<(), String> {
    if !on {
        return match std::fs::remove_file(agent) {
            Err(e) if e.kind() != ErrorKind::NotFound => {
                Err(format!("start at login: delete {}: {e}", agent.display()))
            }
            _ => Ok(()),
        };
    }
    let program = program.to_str().ok_or_else(|| {
        format!(
            "start at login: {} is not UTF-8, which a property list holds",
            program.display()
        )
    })?;
    let dir = agent.parent().unwrap_or(Path::new("."));
    let staged = dir.join(format!(".{}.plist.new", iris_lib::APP_ID));
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&staged, plist(program)))
        .and_then(|()| std::fs::rename(&staged, agent))
        .map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            format!("start at login: write {}: {e}", agent.display())
        })
}

/// The LaunchAgent that runs `program --daemon` once at login, in the
/// GUI session (packaging/macos/dev.iris.app.plist).
fn plist(program: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{program}</string>
		<string>--daemon</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
</dict>
</plist>
"#,
        label = iris_lib::APP_ID,
        program = xml(program),
    )
}

/// `text` with the characters XML reserves as entity references.
fn xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

// WHY: the classes closed here are "Start at login reads on for an
// agent that starts another iris", "a path with an XML character
// writes a property list launchd rejects", and "off fails when there
// is nothing to delete". Not covered: launchd loading the agent at the
// next login.
#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = include_str!("../../../../../packaging/macos/dev.iris.app.plist");

    #[test]
    fn an_agent_starts_this_iris_once_turned_on() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = dir.path().join("LaunchAgents/dev.iris.app.plist");
        let program = Path::new("/Applications/My <Tools> & iris.app/Contents/MacOS/iris");
        assert!(!enabled_at(&agent, program));
        set_at(&agent, program, true).expect("on");
        assert!(enabled_at(&agent, program));
        let text = std::fs::read_to_string(&agent).expect("read");
        assert!(text.contains(
            "<string>/Applications/My &lt;Tools&gt; &amp; iris.app/Contents/MacOS/iris</string>\n\t\t<string>--daemon</string>"
        ), "{text}");
        assert_eq!(
            std::fs::read_dir(agent.parent().unwrap())
                .expect("ls")
                .count(),
            1,
            "a staged copy stayed"
        );
        assert!(!enabled_at(
            &agent,
            Path::new("/Applications/iris.app/Contents/MacOS/iris")
        ));
        set_at(&agent, program, false).expect("off");
        assert!(!agent.exists());
        set_at(&agent, program, false).expect("off again");
    }

    /// The agent this writes and the template in packaging/macos agree
    /// on everything but the program, and a copy of the template reads
    /// as on for the iris it names.
    #[test]
    fn the_agent_is_the_packaged_template() {
        let installed = "/Applications/iris.app/Contents/MacOS/iris";
        let body =
            |text: &str| -> String { text.split_once("<plist").expect("<plist").1.to_string() };
        assert_eq!(body(&plist(installed)), body(TEMPLATE));
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = dir.path().join("dev.iris.app.plist");
        std::fs::write(&agent, TEMPLATE).expect("write");
        assert!(enabled_at(&agent, Path::new(installed)));
    }
}
