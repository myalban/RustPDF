//! Linux isolation, applied by the worker to itself once PDFium is loaded.
//!
//! * **no_new_privs**: nothing the worker runs can gain privileges.
//! * **Landlock**: the worker may read files (fonts, libraries) but can no longer create, write, rename, delete or
//!   make anything executable, and cannot bind or connect TCP sockets (on kernels that support it).
//! * **seccomp**: system calls that open sockets, start programs, trace other processes, load kernel modules, mount
//!   file systems or change namespaces fail with `EPERM`.
//!
//! Each layer is applied on a best-effort basis: the returned text tells which ones are really active.

use landlock::{ABI, Access, AccessFs, AccessNet, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus};
use seccompiler::{BpfProgram, SeccompAction, SeccompFilter, SeccompRule, TargetArch, apply_filter};
use std::collections::BTreeMap;

fn landlock() -> Result<RulesetStatus, String> {
    let abi = ABI::V4;
    let mut ruleset = Ruleset::default().handle_access(AccessFs::from_all(abi)).map_err(|e| e.to_string())?;
    // network (kernel 6.7+): no TCP bind or connect; ignored on older kernels
    ruleset = ruleset.handle_access(AccessNet::BindTcp | AccessNet::ConnectTcp).map_err(|e| e.to_string())?;
    let created = ruleset.create().map_err(|e| e.to_string())?;
    // reads stay possible everywhere; nothing else is allowed anywhere
    let root = PathFd::new("/").map_err(|e| e.to_string())?;
    let created = created.add_rule(PathBeneath::new(root, AccessFs::from_read(abi))).map_err(|e| e.to_string())?;
    created.restrict_self().map(|s| s.ruleset).map_err(|e| e.to_string())
}

fn blocked_syscalls() -> Vec<i64> {
    let mut v = vec![
        libc::SYS_socket,
        libc::SYS_socketpair,
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_accept,
        libc::SYS_accept4,
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_kcmp,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_kexec_load,
        libc::SYS_reboot,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_open_by_handle_at,
        libc::SYS_userfaultfd,
        libc::SYS_mknodat,
    ];
    #[cfg(target_arch = "x86_64")]
    v.extend([libc::SYS_fork, libc::SYS_vfork, libc::SYS_mknod]);
    v
}

fn seccomp() -> Result<(), String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => TargetArch::x86_64,
        "aarch64" => TargetArch::aarch64,
        other => return Err(format!("unsupported architecture {other}")),
    };
    let rules: BTreeMap<i64, Vec<SeccompRule>> = blocked_syscalls().into_iter().map(|s| (s, vec![])).collect();
    let filter = SeccompFilter::new(rules, SeccompAction::Allow, SeccompAction::Errno(libc::EPERM as u32), arch).map_err(|e| e.to_string())?;
    let program: BpfProgram = filter.try_into().map_err(|e: seccompiler::BackendError| e.to_string())?;
    apply_filter(&program).map_err(|e| e.to_string())
}

pub fn lockdown() -> String {
    let mut parts: Vec<String> = vec![];
    // SAFETY: prctl with constant arguments.
    let nnp = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } == 0;
    if nnp {
        parts.push("no_new_privs".into());
    }
    match landlock() {
        Ok(RulesetStatus::FullyEnforced) => parts.push("Landlock (no file writes, no TCP)".into()),
        Ok(RulesetStatus::PartiallyEnforced) => parts.push("Landlock (partial: no file writes)".into()),
        Ok(RulesetStatus::NotEnforced) => parts.push("Landlock unavailable".into()),
        Err(e) => parts.push(format!("Landlock failed: {e}")),
    }
    if nnp {
        match seccomp() {
            Ok(()) => parts.push("seccomp (no sockets, no programs)".into()),
            Err(e) => parts.push(format!("seccomp failed: {e}")),
        }
    }
    format!("Linux: {}", parts.join(" + "))
}
