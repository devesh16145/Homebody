//! The fence on Windows, built on something Windows already has: the **app container**.
//!
//! A program started in an app container with no capabilities
//!
//! - cannot open any connection to the internet or the local network (Windows' own firewall
//!   refuses it: no driver of ours, nothing to install as administrator);
//! - cannot talk to other programs on this computer over the network either;
//! - cannot read your user folder, or anything else that is not explicitly opened to it.
//!
//! Every protected app gets a container of its own. Its way out is the same gate as on the
//! other systems, reached like this: a small relay (this same program) runs *inside* the
//! container and listens where the app expects its gate; it passes everything, untouched, over
//! a named pipe that only this app's container may open, to the real gate outside.
//!
//! ```text
//!  inside the container                         outside
//!  app ── 127.0.0.1:<gate port> ── relay ══ named pipe ══ engine ── gate ── internet
//! ```
//!
//! No setup step and no administrator are needed. What this first version does not do:
//! windowed apps, and interactive terminals (a protected program's output is passed on, but it
//! does not get a console of its own).

#![cfg(windows)]

use crate::config::AppConfig;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW};
use windows_sys::Win32::Security::Isolation::{CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName};
use windows_sys::Win32::Security::{FreeSid, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES};
use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess, InitializeProcThreadAttributeList, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, INFINITE, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

pub const DATA_ROOT: &str = "C:/ProgramData/Homebody";

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// The container's name: one per protected app.
pub fn container_name(app_id: &str) -> String {
    format!("homebody.{}", app_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(48).collect::<String>())
}

pub fn home_for(app: &AppConfig) -> String {
    format!("{DATA_ROOT}/home/{}", app.id)
}

/// The pipe the relay inside an app's container uses to reach its gate.
pub fn gate_pipe(app_id: &str) -> String {
    format!(r"\\.\pipe\homebody-gate-{}", app_id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>())
}

/// An owned security identifier for a container.
pub struct ContainerSid(*mut c_void);
unsafe impl Send for ContainerSid {}

impl ContainerSid {
    /// The container for this app, made if it does not exist yet. Needs no special rights.
    pub fn for_app(app_id: &str) -> Result<ContainerSid, String> {
        let name = wide(&container_name(app_id));
        let display = wide(&format!("Homebody: {app_id}"));
        let mut sid: *mut c_void = std::ptr::null_mut();
        // SAFETY: all pointers are to live, NUL-terminated buffers; `sid` receives an allocation we free in Drop.
        let hr = unsafe { CreateAppContainerProfile(name.as_ptr(), display.as_ptr(), display.as_ptr(), std::ptr::null(), 0, &mut sid) };
        if hr >= 0 && !sid.is_null() {
            return Ok(ContainerSid(sid));
        }
        // Already there (the usual case after the first run): ask for its identifier instead.
        let mut sid: *mut c_void = std::ptr::null_mut();
        let hr2 = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
        if hr2 >= 0 && !sid.is_null() {
            Ok(ContainerSid(sid))
        } else {
            Err(format!("Windows would not make a private space for this app (error {hr:#x}, then {hr2:#x})."))
        }
    }

    /// As text, for access lists: `S-1-15-2-…`.
    pub fn text(&self) -> Result<String, String> {
        let mut out: *mut u16 = std::ptr::null_mut();
        // SAFETY: `self.0` is a valid SID; `out` receives a LocalAlloc'd string we free below.
        if unsafe { ConvertSidToStringSidW(self.0, &mut out) } == 0 || out.is_null() {
            return Err("The private space's identifier could not be read.".into());
        }
        let mut len = 0;
        unsafe {
            while *out.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(out, len));
            LocalFree(out as *mut c_void);
            Ok(s)
        }
    }
}

impl Drop for ContainerSid {
    fn drop(&mut self) {
        // SAFETY: allocated by the system for us in `for_app`.
        unsafe { FreeSid(self.0) };
    }
}

/// Let one container read and run, or also change, something of yours. Uses Windows' own
/// `icacls`, on things you own, so no administrator is needed.
pub fn grant(path: &str, sid_text: &str, change: bool) -> Result<(), String> {
    let p = std::path::Path::new(path);
    let rights = match (p.is_dir(), change) {
        (true, true) => "(OI)(CI)M",
        (true, false) => "(OI)(CI)RX",
        (false, true) => "M",
        (false, false) => "RX",
    };
    let out = std::process::Command::new("icacls").arg(p).arg("/grant").arg(format!("*{sid_text}:{rights}")).output().map_err(|e| format!("icacls could not be run: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{path} could not be opened to the app: {}", String::from_utf8_lossy(&out.stdout).trim()))
    }
}

/// The folders an app needs, opened to its container: a home of its own and its workspace.
pub fn ensure_dirs(app: &AppConfig, sid_text: &str) -> Result<(), String> {
    for dir in [Some(home_for(app)), app.workspace.clone()].into_iter().flatten() {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{dir} could not be made: {e}"))?;
        grant(&dir, sid_text, true)?;
    }
    Ok(())
}

/// A security descriptor for the gate pipe: you (the owner), the system, and this one container.
/// The low label lets a container, which runs at low integrity, write to it at all.
pub struct PipeSecurity {
    descriptor: *mut c_void,
    pub attributes: SECURITY_ATTRIBUTES,
}
unsafe impl Send for PipeSecurity {}

impl PipeSecurity {
    pub fn for_container(sid_text: &str) -> Result<Box<PipeSecurity>, String> {
        let sddl = wide(&format!("D:(A;;GA;;;OW)(A;;GA;;;SY)(A;;GRGW;;;{sid_text})S:(ML;;NW;;;LW)"));
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated; `descriptor` receives a LocalAlloc'd block freed in Drop.
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, std::ptr::null_mut()) } == 0 {
            return Err("The gate's access list could not be made.".into());
        }
        let mut b = Box::new(PipeSecurity { descriptor, attributes: SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 } });
        b.attributes.lpSecurityDescriptor = b.descriptor;
        Ok(b)
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        unsafe { LocalFree(self.descriptor) };
    }
}

/// One Windows command line from separate arguments, quoted the way programs expect.
pub fn command_line(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if !a.is_empty() && !a.chars().any(|c| c == ' ' || c == '\t' || c == '"') {
                return a.clone();
            }
            let mut q = String::from("\"");
            let mut slashes = 0;
            for c in a.chars() {
                match c {
                    '\\' => slashes += 1,
                    '"' => {
                        q.extend(std::iter::repeat('\\').take(slashes * 2 + 1));
                        slashes = 0;
                        q.push('"');
                        continue;
                    }
                    _ => {
                        q.extend(std::iter::repeat('\\').take(slashes));
                        slashes = 0;
                    }
                }
                if c != '\\' {
                    q.push(c);
                }
            }
            q.extend(std::iter::repeat('\\').take(slashes * 2));
            q.push('"');
            q
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A program running in a container.
pub struct Contained {
    process: HANDLE,
}
unsafe impl Send for Contained {}

impl Contained {
    /// Start `args` inside the container, with exactly `env` as its environment, in `cwd`.
    /// Its input, output and errors are ours.
    pub fn spawn(sid: &ContainerSid, args: &[String], env: &[(String, String)], cwd: &str) -> Result<Contained, String> {
        let mut caps = SECURITY_CAPABILITIES { AppContainerSid: sid.0, Capabilities: std::ptr::null_mut(), CapabilityCount: 0, Reserved: 0 };
        let mut size: usize = 0;
        // SAFETY: the documented two-call pattern: first for the size, then to fill our buffer.
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
        let mut list = vec![0u8; size];
        let list_ptr = list.as_mut_ptr() as *mut c_void;
        if unsafe { InitializeProcThreadAttributeList(list_ptr, 1, 0, &mut size) } == 0 {
            return Err("The program could not be prepared (attribute list).".into());
        }
        let ok = unsafe { UpdateProcThreadAttribute(list_ptr, 0, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize, &mut caps as *mut _ as *mut c_void, std::mem::size_of::<SECURITY_CAPABILITIES>(), std::ptr::null_mut(), std::ptr::null_mut()) };
        if ok == 0 {
            unsafe { DeleteProcThreadAttributeList(list_ptr) };
            return Err("The program could not be put in its private space.".into());
        }
        let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si.lpAttributeList = list_ptr;
        // Hand our own input and output to it.
        unsafe {
            si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            si.StartupInfo.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
            si.StartupInfo.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE);
            si.StartupInfo.hStdError = GetStdHandle(STD_ERROR_HANDLE);
            for h in [si.StartupInfo.hStdInput, si.StartupInfo.hStdOutput, si.StartupInfo.hStdError] {
                if !h.is_null() {
                    SetHandleInformation(h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
                }
            }
        }
        // The environment block: NAME=value, each ended by NUL, the whole ended by another.
        let mut block: Vec<u16> = Vec::new();
        for (k, v) in env {
            block.extend(std::ffi::OsStr::new(&format!("{k}={v}")).encode_wide());
            block.push(0);
        }
        block.push(0);
        let mut cmd = wide(&command_line(args));
        let dir = wide(&cwd.replace('/', "\\"));
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: every pointer outlives the call; `cmd` is mutable as CreateProcessW requires.
        let made = unsafe { CreateProcessW(std::ptr::null(), cmd.as_mut_ptr(), std::ptr::null(), std::ptr::null(), 1, EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT, block.as_ptr() as *const c_void, dir.as_ptr(), &si.StartupInfo, &mut pi) };
        let err = std::io::Error::last_os_error();
        unsafe { DeleteProcThreadAttributeList(list_ptr) };
        if made == 0 {
            return Err(format!("{} could not be started in its private space: {err}", args.first().map(String::as_str).unwrap_or("The program")));
        }
        unsafe { CloseHandle(pi.hThread) };
        Ok(Contained { process: pi.hProcess })
    }

    /// Wait for it to end; its exit code.
    pub fn wait(&self) -> i32 {
        let mut code: u32 = 1;
        unsafe {
            WaitForSingleObject(self.process, INFINITE);
            GetExitCodeProcess(self.process, &mut code);
        }
        code as i32
    }

    pub fn kill(&self) {
        unsafe { TerminateProcess(self.process, 1) };
    }
}

impl Drop for Contained {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.process) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_quoted_the_way_windows_programs_read_them() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(command_line(&v(&["a.exe", "plain", "two words", ""])), "a.exe plain \"two words\" \"\"");
        assert_eq!(command_line(&v(&["say \"hi\""])), "\"say \\\"hi\\\"\"");
        assert_eq!(command_line(&v(&["C:\\dir with space\\"])), "\"C:\\dir with space\\\\\"");
    }

    #[test]
    fn a_container_can_be_made_and_named_without_special_rights() {
        let sid = ContainerSid::for_app("unit-test").expect("container");
        let text = sid.text().expect("sid text");
        assert!(text.starts_with("S-1-15-2-"), "{text}");
        // The same name gives the same container.
        assert_eq!(ContainerSid::for_app("unit-test").unwrap().text().unwrap(), text);
    }

    #[test]
    fn a_program_in_a_container_runs_but_cannot_reach_the_internet_or_your_files() {
        let sid = ContainerSid::for_app("unit-test").expect("container");
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let sys = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let env = vec![("SystemRoot".to_string(), sys.clone()), ("PATH".to_string(), format!("{sys}\\System32"))];
        let run = |args: &[&str]| Contained::spawn(&sid, &v(args), &env, &format!("{sys}\\System32")).map(|c| c.wait());
        // Control: a harmless command runs and says it succeeded.
        assert_eq!(run(&["cmd.exe", "/c", "exit", "0"]), Ok(0));
        assert_eq!(run(&["cmd.exe", "/c", "exit", "7"]), Ok(7));
        // Straight to the internet: refused by Windows itself.
        let direct = run(&["curl.exe", "-sS", "-m", "8", "-o", "NUL", "https://1.1.1.1/"]).unwrap();
        assert_ne!(direct, 0, "a contained program reached the internet directly");
        // Your own folder: closed.
        let home = std::env::var("USERPROFILE").unwrap();
        let secret = format!("{home}\\homebody-unit-test.txt");
        std::fs::write(&secret, "secret").unwrap();
        let read = run(&["cmd.exe", "/c", "type", &secret]).unwrap();
        let _ = std::fs::remove_file(&secret);
        assert_ne!(read, 0, "a contained program read a file in your user folder");
    }
}
