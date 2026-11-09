//! macOS: Finder Quick Actions (Automator service workflows) in
//! `~/Library/Services`, one per verb, for PDFs and folders. They appear
//! under Quick Actions / Services in Finder's context menu. Code only this
//! release (not validated on a Mac): see `playbooks/release.md`.
//!
//! Each workflow is a "Run Shell Script" action that starts the app
//! detached, so Finder's progress gear does not spin while the window is
//! open. A signed Finder Action Extension (`.appex`) is the later upgrade.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::Path;

use super::{FileEntry, MARKER, PRODUCT, Verb};

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn workflow_name(verb: Verb) -> String {
    match verb {
        Verb::RenameNow => format!("{PRODUCT} - Rename now.workflow"),
        Verb::Preview => format!("{PRODUCT} - Preview.workflow"),
    }
}

/// `Contents/Info.plist`: the service definition (menu title, input types).
pub fn info_plist(verb: Verb) -> String {
    let title = match verb {
        Verb::RenameNow => format!("{PRODUCT}: Rename now"),
        Verb::Preview => format!("{PRODUCT}: Preview"),
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- {MARKER} -->
<plist version="1.0">
<dict>
	<key>NSServices</key>
	<array>
		<dict>
			<key>NSIconName</key>
			<string>NSActionTemplate</string>
			<key>NSMenuItem</key>
			<dict>
				<key>default</key>
				<string>{title}</string>
			</dict>
			<key>NSMessage</key>
			<string>runWorkflowAsService</string>
			<key>NSRequiredContext</key>
			<dict>
				<key>NSApplicationIdentifier</key>
				<string>com.apple.finder</string>
			</dict>
			<key>NSSendFileTypes</key>
			<array>
				<string>com.adobe.pdf</string>
				<string>public.folder</string>
			</array>
		</dict>
	</array>
</dict>
</plist>
"#,
        title = xml(&title),
    )
}

/// Shell command of the workflow (input passed as arguments).
pub fn shell_command(exe: &str, verb: Verb) -> String {
    let mut cmd = vec![super::shell_quote(exe)];
    cmd.extend(verb.args().iter().map(|a| a.to_string()));
    format!("nohup {} -- \"$@\" >/dev/null 2>&1 &", cmd.join(" "))
}

/// `Contents/document.wflow`: one "Run Shell Script" action.
pub fn document_wflow(exe: &str, verb: Verb) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- {MARKER} -->
<plist version="1.0">
<dict>
	<key>AMApplicationBuild</key>
	<string>523</string>
	<key>AMApplicationVersion</key>
	<string>2.10</string>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<true/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>2.0.3</string>
				<key>AMApplication</key>
				<array>
					<string>Automator</string>
				</array>
				<key>AMParameterProperties</key>
				<dict>
					<key>COMMAND_STRING</key>
					<dict/>
					<key>CheckedForUserDefaultShell</key>
					<dict/>
					<key>inputMethod</key>
					<dict/>
					<key>shell</key>
					<dict/>
					<key>source</key>
					<dict/>
				</dict>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>{cmd}</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>1</integer>
					<key>shell</key>
					<string>/bin/sh</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>CFBundleVersion</key>
				<string>2.0.3</string>
				<key>CanShowSelectedItemsWhenRun</key>
				<false/>
				<key>CanShowWhenRun</key>
				<true/>
				<key>Category</key>
				<array>
					<string>AMCategoryUtilities</string>
				</array>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>6E7C1D4A-0001-4B6B-9C1E-A0A0A0A0A0A1</string>
				<key>OutputUUID</key>
				<string>6E7C1D4A-0002-4B6B-9C1E-A0A0A0A0A0A2</string>
				<key>UUID</key>
				<string>6E7C1D4A-0003-4B6B-9C1E-A0A0A0A0A0A3</string>
				<key>isViewVisible</key>
				<integer>1</integer>
			</dict>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>serviceInputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject</string>
		<key>serviceOutputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>serviceProcessesInput</key>
		<integer>0</integer>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>
"#,
        cmd = xml(&shell_command(exe, verb)),
    )
}

/// Both workflow bundles under `services` (`~/Library/Services`).
pub fn entries(services: &Path, exe: &str) -> Vec<FileEntry> {
    let mut v = Vec::new();
    for verb in Verb::ALL {
        let dir = services.join(workflow_name(verb)).join("Contents");
        for (name, body) in [
            ("Info.plist", info_plist(verb)),
            ("document.wflow", document_wflow(exe, verb)),
        ] {
            v.push(FileEntry {
                kind: "quick-action",
                path: dir.join(name),
                bytes: body.into_bytes(),
                executable: false,
                owned_by_name: false,
            });
        }
    }
    v
}

#[cfg(target_os = "macos")]
pub fn run(action: super::Action, exe: &Path) -> autorename_core::Result<super::Report> {
    use super::{Action, OwnedLog, Report, install_files, io_err, remove_files, status_files};
    let exe_s = exe.to_str().ok_or_else(|| autorename_core::Error::Usage {
        message: format!("executable path is not UTF-8: {}", exe.display()),
        hint: None,
    })?;
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| autorename_core::Error::General("HOME is not set".into()))?;
    let services = home.join("Library/Services");
    let all = entries(&services, exe_s);
    let mut log = OwnedLog::load(
        home.join("Library/Application Support/autorename-pdf/integration-owned.txt"),
    );
    let items = match action {
        Action::Status => status_files(&all, &log),
        Action::Install => {
            let items = install_files(&all, &mut log).map_err(|e| io_err(&services, e))?;
            log.save().map_err(|e| io_err(&services, e))?;
            items
        }
        Action::Remove => {
            let items = remove_files(&all, &mut log).map_err(|e| io_err(&services, e))?;
            log.prune().map_err(|e| io_err(&services, e))?;
            items
        }
    };
    if action != Action::Status {
        let pbs = Path::new("/System/Library/CoreServices/pbs");
        if pbs.is_file() {
            let _ = std::process::Command::new(pbs).arg("-update").status();
        }
    }
    Ok(Report {
        platform: "macos",
        scope: "user",
        command: exe_s.into(),
        items,
        notes: vec![
            "Finder: right-click > Quick Actions (or Services). Enable them in System Settings > Keyboard > Keyboard Shortcuts > Services if hidden."
                .into(),
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_command_golden() {
        assert_eq!(
            shell_command(
                "/Applications/AutoRename-PDF.app/Contents/MacOS/autorename-pdf",
                Verb::RenameNow
            ),
            "nohup /Applications/AutoRename-PDF.app/Contents/MacOS/autorename-pdf gui --rename-now -- \"$@\" >/dev/null 2>&1 &"
        );
    }

    #[test]
    fn workflow_is_marked_and_escaped() {
        let w = document_wflow("/A & B/autorename-pdf", Verb::Preview);
        assert!(w.contains(MARKER));
        assert!(w.contains("<string>nohup '/A &amp; B/autorename-pdf' gui -- &quot;$@&quot; &gt;/dev/null 2&gt;&amp;1 &amp;</string>"));
        let p = info_plist(Verb::RenameNow);
        assert!(p.contains("<string>AutoRename-PDF: Rename now</string>"));
        assert!(p.contains("<string>com.adobe.pdf</string>"));
        let e = entries(Path::new("/Users/me/Library/Services"), "/x");
        assert_eq!(e.len(), 4);
        assert_eq!(
            e[0].path,
            Path::new(
                "/Users/me/Library/Services/AutoRename-PDF - Rename now.workflow/Contents/Info.plist"
            )
        );
    }
}
