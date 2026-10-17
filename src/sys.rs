//! System integration: .pdf file association, opening folders, files and links.
//! Windows: registry (HKCU) + Explorer; Linux: `xdg-open` + a user `.desktop` file; macOS: `open`.

use std::path::Path;
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn run(cmd: &mut Command) -> std::io::Result<std::process::ExitStatus> {
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.status()
}

/// Program that opens a file, folder or URL with the default application of the platform.
#[cfg(target_os = "macos")]
const OPENER: &str = "open";
#[cfg(all(unix, not(target_os = "macos")))]
const OPENER: &str = "xdg-open";

pub fn open_folder(p: &Path) {
    #[cfg(windows)]
    {
        let _ = Command::new("explorer").arg(p).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new(OPENER).arg(p).spawn();
    }
}

pub fn open_file(p: &Path) {
    #[cfg(windows)]
    {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", &p.to_string_lossy()]);
        let _ = run(&mut c);
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new(OPENER).arg(p).spawn();
    }
}

pub fn open_url(u: &str) {
    #[cfg(windows)]
    {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", u]);
        let _ = run(&mut c);
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new(OPENER).arg(u).spawn();
    }
}

/// Content of the freedesktop `.desktop` entry that declares RustPDF as a PDF viewer (Linux).
#[cfg_attr(windows, allow(dead_code))]
pub fn desktop_entry(exe: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName=RustPDF\nComment=PDF editor\nExec=\"{exe}\" %f\nTerminal=false\nCategories=Office;Viewer;\nMimeType=application/pdf;\nStartupNotify=true\n")
}

/// `reg add` commands needed to declare RustPDF as an application able to open .pdf files.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn association_commands(exe: &str) -> Vec<Vec<String>> {
    let open_cmd = format!("\"{exe}\" \"%1\"");
    let v = |args: &[&str]| args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    vec![
        v(&["add", r"HKCU\Software\Classes\RustPDF.Document", "/ve", "/d", "PDF Document (RustPDF)", "/f"]),
        v(&["add", r"HKCU\Software\Classes\RustPDF.Document\DefaultIcon", "/ve", "/d", &format!("\"{exe}\",0"), "/f"]),
        vec!["add".into(), r"HKCU\Software\Classes\RustPDF.Document\shell\open\command".into(), "/ve".into(), "/d".into(), open_cmd.clone(), "/f".into()],
        v(&["add", r"HKCU\Software\Classes\.pdf\OpenWithProgids", "/v", "RustPDF.Document", "/t", "REG_NONE", "/f"]),
        vec!["add".into(), r"HKCU\Software\Classes\Applications\rustpdf.exe\shell\open\command".into(), "/ve".into(), "/d".into(), open_cmd, "/f".into()],
        v(&["add", r"HKCU\Software\RustPDF\Capabilities", "/v", "ApplicationName", "/d", "RustPDF", "/f"]),
        v(&["add", r"HKCU\Software\RustPDF\Capabilities", "/v", "ApplicationDescription", "/d", "PDF editor", "/f"]),
        v(&["add", r"HKCU\Software\RustPDF\Capabilities\FileAssociations", "/v", ".pdf", "/d", "RustPDF.Document", "/f"]),
        v(&["add", r"HKCU\Software\RegisteredApplications", "/v", "RustPDF", "/d", r"Software\RustPDF\Capabilities", "/f"]),
    ]
}


/// Registers the application as a .pdf handler.
/// Windows: declares it in HKCU (no administrator rights) then opens the "Default apps" settings, because
/// Windows requires the user to confirm the choice. Linux: installs a user `.desktop` file and runs `xdg-mime`.
pub fn associate_pdf() -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.to_string_lossy().into_owned();
        for args in association_commands(&exe) {
            let st = run(Command::new("reg").args(&args)).map_err(|e| e.to_string())?;
            if !st.success() {
                return Err(format!("reg {} failed", args.join(" ")));
            }
        }
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", "ms-settings:defaultapps"]);
        let _ = run(&mut c);
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        Err("on macOS: select a PDF in Finder, Get Info (Cmd+I), Open with RustPDF, Change All…".into())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        let dir = std::path::PathBuf::from(home).join(".local/share/applications");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("rustpdf.desktop"), desktop_entry(&exe.to_string_lossy())).map_err(|e| e.to_string())?;
        let st = Command::new("xdg-mime").args(["default", "rustpdf.desktop", "application/pdf"]).status().map_err(|e| e.to_string())?;
        if st.success() { Ok(()) } else { Err("xdg-mime failed".into()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn association_commands_quote_the_exe_path() {
        let cmds = association_commands(r"C:\Program Files\RustPDF\rustpdf.exe");
        assert!(cmds.len() >= 8);
        let open = cmds.iter().find(|c| c.iter().any(|a| a.contains(r"shell\open\command"))).unwrap();
        let expected = r#""C:\Program Files\RustPDF\rustpdf.exe" "%1""#;
        assert!(open.iter().any(|a| a == expected));
        assert!(cmds.iter().all(|c| c[0] == "add" && c.last().map(|a| a == "/f").unwrap_or(false)));
    }

    #[test]
    fn desktop_entry_declares_the_pdf_mime_type() {
        let d = desktop_entry("/opt/rustpdf/rustpdf");
        assert!(d.contains("MimeType=application/pdf;"));
        assert!(d.contains("Exec=\"/opt/rustpdf/rustpdf\" %f"));
    }
}
