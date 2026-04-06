//! thane-sandbox-exec — helper binary for macOS process sandboxing.
//!
//! Replaces the deprecated `sandbox-exec` CLI tool. This binary:
//! 1. Reads an SBPL profile from `THANE_SANDBOX_PROFILE` env var
//! 2. Applies it via `sandbox_init_with_parameters()` (Seatbelt C API)
//! 3. Applies resource limits from `THANE_SANDBOX_MAX_*` env vars
//! 4. Cleans up sandbox env vars (so the shell doesn't inherit them)
//! 5. Execs the target command (argv[1..])
//!
//! Usage: thane-sandbox-exec <shell> [args...]
//!   Env: THANE_SANDBOX_PROFILE=<SBPL profile string>
//!        THANE_SANDBOX_MAX_FILES=<max open files>  (optional)
//!        THANE_SANDBOX_MAX_FSIZE=<max file size>   (optional)
//!        THANE_SANDBOX_MAX_CPU=<cpu seconds>       (optional)

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("thane-sandbox-exec: this binary is only functional on macOS");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() {
    macos_main();
}

#[cfg(target_os = "macos")]
fn macos_main() {
    use std::env;
    use std::ffi::{CString, CStr};
    use std::ptr;

    // Seatbelt C API — stable but undocumented. Used by Chrome, Firefox, and Apple's own tools.
    unsafe extern "C" {
        fn sandbox_init_with_parameters(
            profile: *const libc::c_char,
            flags: u64,
            parameters: *const *const libc::c_char,
            errorbuf: *mut *mut libc::c_char,
        ) -> libc::c_int;

        fn sandbox_free_error(errorbuf: *mut libc::c_char);
    }

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("thane-sandbox-exec: no command specified");
        eprintln!("Usage: thane-sandbox-exec <shell> [args...]");
        std::process::exit(1);
    }

    // 1. Read and apply SBPL profile
    let profile = match env::var("THANE_SANDBOX_PROFILE") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            eprintln!("thane-sandbox-exec: THANE_SANDBOX_PROFILE not set or empty");
            std::process::exit(1);
        }
    };

    let profile_cstr = CString::new(profile).unwrap_or_else(|_| {
        eprintln!("thane-sandbox-exec: profile contains null byte");
        std::process::exit(1);
    });

    let mut errorbuf: *mut libc::c_char = ptr::null_mut();
    let ret = unsafe {
        sandbox_init_with_parameters(profile_cstr.as_ptr(), 0, ptr::null(), &mut errorbuf)
    };

    if ret != 0 {
        let err_msg = if !errorbuf.is_null() {
            let msg = unsafe { CStr::from_ptr(errorbuf) }.to_string_lossy().to_string();
            unsafe { sandbox_free_error(errorbuf) };
            msg
        } else {
            "unknown error".to_string()
        };
        eprintln!("thane-sandbox-exec: sandbox_init failed: {err_msg}");
        std::process::exit(1);
    }

    // 2. Apply resource limits
    if let Ok(val) = env::var("THANE_SANDBOX_MAX_FILES") {
        if let Ok(limit) = val.parse::<u64>() {
            let rlim = libc::rlimit { rlim_cur: limit, rlim_max: limit };
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &rlim); }
        }
    }
    if let Ok(val) = env::var("THANE_SANDBOX_MAX_FSIZE") {
        if let Ok(limit) = val.parse::<u64>() {
            let rlim = libc::rlimit { rlim_cur: limit, rlim_max: limit };
            unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &rlim); }
        }
    }
    if let Ok(val) = env::var("THANE_SANDBOX_MAX_CPU") {
        if let Ok(limit) = val.parse::<u64>() {
            // Soft limit sends SIGXCPU; hard limit (+ 10s grace) sends SIGKILL
            let rlim = libc::rlimit { rlim_cur: limit, rlim_max: limit + 10 };
            unsafe { libc::setrlimit(libc::RLIMIT_CPU, &rlim); }
        }
    }

    // 3. Clean up sandbox env vars — shell shouldn't inherit the profile.
    // SAFETY: We are single-threaded at this point (before exec), so removing
    // env vars is safe. The child process (exec'd shell) won't see them.
    unsafe {
        env::remove_var("THANE_SANDBOX_PROFILE");
        env::remove_var("THANE_SANDBOX_MAX_FILES");
        env::remove_var("THANE_SANDBOX_MAX_FSIZE");
        env::remove_var("THANE_SANDBOX_MAX_CPU");
    }

    // 4. Exec the target command
    let cmd = CString::new(args[1].clone()).unwrap_or_else(|_| {
        eprintln!("thane-sandbox-exec: command contains null byte");
        std::process::exit(1);
    });

    let c_args: Vec<CString> = args[1..]
        .iter()
        .map(|a| CString::new(a.as_bytes()).unwrap_or_else(|_| {
            eprintln!("thane-sandbox-exec: argument contains null byte");
            std::process::exit(1);
        }))
        .collect();

    let c_arg_ptrs: Vec<*const libc::c_char> = c_args.iter().map(|a| a.as_ptr()).chain(std::iter::once(ptr::null())).collect();

    unsafe {
        libc::execvp(cmd.as_ptr(), c_arg_ptrs.as_ptr());
    }

    // If execvp returns, it failed
    let errno = std::io::Error::last_os_error();
    eprintln!("thane-sandbox-exec: exec failed: {errno}");
    std::process::exit(1);
}
