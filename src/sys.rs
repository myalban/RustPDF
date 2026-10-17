//! System integration (Windows): .pdf file association, opening folders, etc.

use std::path::Path;
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn run(cmd: &mut Command) -> std::io::Result<std::process::ExitStatus> {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.status()
}

pub fn open_folder(p: &Path) {
    let _ = Command::new("explorer").arg(p).spawn();
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
        let _ = Command::new("xdg-open").arg(p).spawn();
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
        let _ = Command::new("xdg-open").arg(u).spawn();
    }
}

/// `reg add` commands needed to declare RustPDF as an application able to open .pdf files.
pub fn association_commands(exe: &str) -> Vec<Vec<String>> {
    let open_cmd = format!("\"{exe}\" \"%1\"");
    let v = |args: &[&str]| args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    vec![
        v(&["add", r"HKCU\Software\Classes\RustPDF.Document", "/ve", "/d", "Document PDF (RustPDF)", "/f"]),
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

/// Declares the application to Windows (in HKCU, without administrator rights) then opens the
/// "Default apps" settings: Windows requires the user to confirm the choice themselves.
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
    #[cfg(not(windows))]
    {
        Err("not supported on this system".into())
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
}
