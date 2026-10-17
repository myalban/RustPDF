//! Windows isolation: the worker runs in an AppContainer inside a job object.
//!
//! * **AppContainer**: a process in an AppContainer has no access to the user's files (only to the folders that are
//!   explicitly opened to it, here the folder where the worker program is staged), no access to the network and to
//!   most of the registry and of the other processes.
//! * **Job object**: no child process, a memory cap, no access to the clipboard, the desktop, other windows or the
//!   display settings, and the worker dies with the application.
//! * **Mitigations**: no dynamic code generation (a classic way to run an attacker's shellcode), no extension
//!   points, strict handle checks, no images from remote or low-integrity locations, high-entropy ASLR.

use super::Transport;
use std::ffi::{OsStr, c_void};
use std::fs::File;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, LocalFree, SetHandleInformation, TRUE};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::Isolation::{CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName};
use windows_sys::Win32::Security::{GetTokenInformation, PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES, TOKEN_QUERY, TokenIsAppContainer};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD, JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess, InitializeProcThreadAttributeList, OpenProcessToken,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, TerminateProcess,
    UpdateProcThreadAttribute,
};

/// Name of the AppContainer profile (created on first use, a few kilobytes in the user's profile).
const APP_CONTAINER: &str = "RustPDF.ProtectedViewer";
/// Memory the worker may use: the document itself plus the page bitmaps.
const MEMORY_LIMIT: usize = 3 * 1024 * 1024 * 1024;

// Process creation mitigation policy flags (winbase.h)
const MIT_HEAP_TERMINATE: u64 = 1 << 12;
const MIT_BOTTOM_UP_ASLR: u64 = 1 << 16;
const MIT_HIGH_ENTROPY_ASLR: u64 = 1 << 20;
const MIT_STRICT_HANDLE_CHECKS: u64 = 1 << 24;
const MIT_EXTENSION_POINT_DISABLE: u64 = 1 << 32;
const MIT_PROHIBIT_DYNAMIC_CODE: u64 = 1 << 36;
const MIT_IMAGE_NO_REMOTE: u64 = 1 << 52;
const MIT_IMAGE_NO_LOW_LABEL: u64 = 1 << 56;
const MIT_IMAGE_PREFER_SYSTEM32: u64 = 1 << 60;

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

fn last_error(what: &str) -> String {
    format!("{what} failed (Windows error {})", std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
}

/// The AppContainer SID, created if needed.
fn app_container_sid() -> Result<PSID, String> {
    let name = wide(APP_CONTAINER);
    let display = wide("RustPDF protected viewer");
    let description = wide("Isolated process that renders untrusted PDF documents");
    let mut sid: PSID = null_mut();
    // SAFETY: valid NUL-terminated strings and an out pointer.
    unsafe {
        let hr = CreateAppContainerProfile(name.as_ptr(), display.as_ptr(), description.as_ptr(), null(), 0, &mut sid);
        if hr == 0 {
            return Ok(sid);
        }
        if hr as u32 == 0x800700B7 {
            // the profile already exists
            let hr2 = DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid);
            if hr2 == 0 {
                return Ok(sid);
            }
            return Err(format!("DeriveAppContainerSid failed (0x{:08x})", hr2 as u32));
        }
        Err(format!("CreateAppContainerProfile failed (0x{:08x})", hr as u32))
    }
}

fn sid_to_string(sid: PSID) -> Result<String, String> {
    let mut p: *mut u16 = null_mut();
    // SAFETY: the returned buffer is read up to its terminating NUL and freed with LocalFree.
    unsafe {
        if ConvertSidToStringSidW(sid, &mut p) == 0 {
            return Err(last_error("ConvertSidToStringSid"));
        }
        let mut len = 0;
        while *p.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
        LocalFree(p as *mut c_void);
        Ok(s)
    }
}

fn local_app_data() -> Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).ok_or_else(|| "LOCALAPPDATA is not set".to_string())
}

/// Copies `src` to `dst` unless an identical copy is already there.
fn copy_if_changed(src: &Path, dst: &Path) -> Result<(), String> {
    let sm = std::fs::metadata(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let dm = std::fs::metadata(dst);
    let same = dm.as_ref().map(|d| d.len() == sm.len() && d.modified().ok() >= sm.modified().ok()).unwrap_or(false);
    if same {
        return Ok(());
    }
    std::fs::copy(src, dst).map(|_| ()).map_err(|e| format!("copying {}: {e}", src.display()))
}

/// Prepares the folder from which the worker runs: the AppContainer may read it (and nothing else of the user's
/// disk). Returns the path of the staged executable and the folder.
fn stage(sid_text: &str) -> Result<(PathBuf, PathBuf), String> {
    let dir = local_app_data()?.join("RustPDF").join("worker");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let marker = dir.join(format!(".acl-{}", sid_text.replace('-', "")));
    if !marker.exists() {
        use std::os::windows::process::CommandExt;
        let grant = format!("*{sid_text}:(OI)(CI)RX");
        let st = std::process::Command::new("icacls").arg(&dir).args(["/grant", &grant, "/T", "/C", "/Q"]).creation_flags(0x0800_0000).status().map_err(|e| format!("icacls: {e}"))?;
        if !st.success() {
            return Err("icacls could not open the worker folder to the AppContainer".into());
        }
        std::fs::write(&marker, b"").map_err(|e| e.to_string())?;
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = exe.parent().ok_or("no executable folder")?.to_path_buf();
    let staged_exe = dir.join("rustpdf-worker.exe");
    copy_if_changed(&exe, &staged_exe)?;
    let dll = exe_dir.join("pdfium.dll");
    let dll = if dll.exists() || !cfg!(debug_assertions) { dll } else { PathBuf::from("pdfium.dll") };
    copy_if_changed(&dll, &dir.join("pdfium.dll"))?;
    Ok((staged_exe, dir))
}

struct Handles {
    process: HANDLE,
    job: HANDLE,
    done: AtomicBool,
}

// SAFETY: Windows handles can be used from any thread.
unsafe impl Send for Handles {}
unsafe impl Sync for Handles {}

impl Handles {
    fn kill(&self) {
        if !self.done.swap(true, Ordering::SeqCst) {
            // SAFETY: the handles are valid until this single close.
            unsafe {
                TerminateProcess(self.process, 1);
                CloseHandle(self.job);
                CloseHandle(self.process);
            }
        }
    }
}

fn make_job(process: HANDLE) -> Result<HANDLE, String> {
    // SAFETY: plain Win32 calls with correctly sized structures.
    unsafe {
        let job = CreateJobObjectW(null(), null());
        if job.is_null() {
            return Err(last_error("CreateJobObject"));
        }
        let mut ext: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        ext.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        ext.BasicLimitInformation.ActiveProcessLimit = 1;
        ext.ProcessMemoryLimit = MEMORY_LIMIT;
        if SetInformationJobObject(job, JobObjectExtendedLimitInformation, &ext as *const _ as *const c_void, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32) == 0 {
            return Err(last_error("SetInformationJobObject(limits)"));
        }
        let ui_flags = JOB_OBJECT_UILIMIT_DESKTOP
            | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
            | JOB_OBJECT_UILIMIT_EXITWINDOWS
            | JOB_OBJECT_UILIMIT_GLOBALATOMS
            | JOB_OBJECT_UILIMIT_HANDLES
            | JOB_OBJECT_UILIMIT_READCLIPBOARD
            | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
            | JOB_OBJECT_UILIMIT_WRITECLIPBOARD;
        let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS { UIRestrictionsClass: ui_flags };
        if SetInformationJobObject(job, JobObjectBasicUIRestrictions, &ui as *const _ as *const c_void, std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32) == 0 {
            return Err(last_error("SetInformationJobObject(ui)"));
        }
        if AssignProcessToJobObject(job, process) == 0 {
            return Err(last_error("AssignProcessToJobObject"));
        }
        Ok(job)
    }
}

pub fn spawn() -> Result<Transport, String> {
    let sid = app_container_sid()?;
    let sid_text = sid_to_string(sid)?;
    let (exe, dir) = stage(&sid_text)?;

    // SAFETY: the sequence below follows the documented use of extended startup information; every handle and
    // buffer outlives the calls that use it.
    unsafe {
        // pipes: the worker's ends are inheritable, ours are not
        let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: null_mut(), bInheritHandle: TRUE };
        let (mut in_r, mut in_w, mut out_r, mut out_w): (HANDLE, HANDLE, HANDLE, HANDLE) = (null_mut(), null_mut(), null_mut(), null_mut());
        if CreatePipe(&mut in_r, &mut in_w, &sa, 0) == 0 || CreatePipe(&mut out_r, &mut out_w, &sa, 0) == 0 {
            return Err(last_error("CreatePipe"));
        }
        SetHandleInformation(in_w, HANDLE_FLAG_INHERIT, 0);
        SetHandleInformation(out_r, HANDLE_FLAG_INHERIT, 0);
        let nul = std::fs::OpenOptions::new().write(true).open("NUL").map_err(|e| e.to_string())?;
        let nul_h = nul.as_raw_handle() as HANDLE;
        SetHandleInformation(nul_h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
        let mut inherit: [HANDLE; 3] = [in_r, out_w, nul_h];

        // attribute list
        let mut caps = SECURITY_CAPABILITIES { AppContainerSid: sid, Capabilities: null_mut(), CapabilityCount: 0, Reserved: 0 };
        let mitigation: u64 = MIT_HEAP_TERMINATE
            | MIT_BOTTOM_UP_ASLR
            | MIT_HIGH_ENTROPY_ASLR
            | MIT_STRICT_HANDLE_CHECKS
            | MIT_EXTENSION_POINT_DISABLE
            | MIT_PROHIBIT_DYNAMIC_CODE
            | MIT_IMAGE_NO_REMOTE
            | MIT_IMAGE_NO_LOW_LABEL
            | MIT_IMAGE_PREFER_SYSTEM32;

        let mut size = 0usize;
        InitializeProcThreadAttributeList(null_mut(), 3, 0, &mut size);
        let mut buf = vec![0u64; size.div_ceil(8) + 1];
        let list = buf.as_mut_ptr() as *mut c_void;
        if InitializeProcThreadAttributeList(list, 3, 0, &mut size) == 0 {
            return Err(last_error("InitializeProcThreadAttributeList"));
        }
        let ok = UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize, &mut caps as *mut _ as *const c_void, std::mem::size_of::<SECURITY_CAPABILITIES>(), null_mut(), null())
            != 0
            && UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, inherit.as_mut_ptr() as *const c_void, std::mem::size_of_val(&inherit), null_mut(), null()) != 0
            && UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY as usize, &mitigation as *const _ as *const c_void, 8, null_mut(), null()) != 0;
        if !ok {
            let e = last_error("UpdateProcThreadAttribute");
            DeleteProcThreadAttributeList(list);
            return Err(e);
        }

        let mut si: STARTUPINFOEXW = std::mem::zeroed();
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = in_r;
        si.StartupInfo.hStdOutput = out_w;
        si.StartupInfo.hStdError = nul_h;
        si.lpAttributeList = list;
        let app = wide(&exe);
        let mut cmd = wide(format!("\"{}\" --pdf-worker", exe.display()));
        let cwd = wide(&dir);
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let created = CreateProcessW(
            app.as_ptr(),
            cmd.as_mut_ptr(),
            null(),
            null(),
            TRUE,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_NO_WINDOW,
            null(),
            cwd.as_ptr(),
            &si as *const STARTUPINFOEXW as *const STARTUPINFOW,
            &mut pi,
        );
        let err = if created == 0 { Some(last_error("CreateProcess")) } else { None };
        DeleteProcThreadAttributeList(list);
        // the worker's ends of the pipes now belong to the worker
        CloseHandle(in_r);
        CloseHandle(out_w);
        drop(nul);
        if let Some(e) = err {
            CloseHandle(in_w);
            CloseHandle(out_r);
            return Err(e);
        }
        let job = match make_job(pi.hProcess) {
            Ok(j) => j,
            Err(e) => {
                TerminateProcess(pi.hProcess, 1);
                CloseHandle(pi.hProcess);
                CloseHandle(pi.hThread);
                CloseHandle(in_w);
                CloseHandle(out_r);
                return Err(e);
            }
        };
        ResumeThread(pi.hThread);
        CloseHandle(pi.hThread);
        let handles = Arc::new(Handles { process: pi.hProcess, job, done: AtomicBool::new(false) });
        let h = handles.clone();
        let h2 = handles.clone();
        Ok(Transport { input: Box::new(File::from_raw_handle(in_w as _)), output: Box::new(File::from_raw_handle(out_r as _)), kill: Arc::new(move || h.kill()), exit_code: Arc::new(move || h2.exit_code()) })
    }
}

/// What the worker reports about itself.
pub fn describe_self() -> String {
    // SAFETY: queries the token of the current process.
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) != 0 {
            let (mut flag, mut returned) = (0u32, 0u32);
            let ok = GetTokenInformation(token, TokenIsAppContainer, &mut flag as *mut u32 as *mut c_void, 4, &mut returned) != 0;
            CloseHandle(token);
            if ok && flag != 0 {
                return "Windows AppContainer + job object: no files, no network, no other programs".into();
            }
        }
    }
    "none".into()
}

impl Handles {
    fn exit_code(&self) -> Option<i64> {
        if self.done.load(Ordering::SeqCst) {
            return None;
        }
        let mut code = 0u32;
        // SAFETY: valid process handle (not closed yet, see `done`).
        let ok = unsafe { GetExitCodeProcess(self.process, &mut code) } != 0;
        // STILL_ACTIVE
        if ok && code != 259 { Some(code as i64) } else { None }
    }
}
