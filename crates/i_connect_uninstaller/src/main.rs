#![windows_subsystem = "windows"]

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, WPARAM};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteKeyW, RegDeleteValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    KEY_SET_VALUE,
};
use windows::Win32::UI::Shell::{SHGetFolderPathW, CSIDL_DESKTOPDIRECTORY, CSIDL_PROGRAMS};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, MessageBoxW, SendMessageW, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK,
    MB_YESNO, WM_COMMAND,
};

const CREATE_NO_WINDOW: u32 = 0x08000000;

fn get_shell_folder(csidl: i32) -> Option<PathBuf> {
    let mut buf = [0u16; 260];
    let hr = unsafe { SHGetFolderPathW(HWND::default(), csidl, None, 0, &mut buf) };
    if hr.is_ok() {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
    } else {
        None
    }
}

fn main() {
    unsafe {
        let msg = HSTRING::from("Are you sure you want to completely uninstall I Connect Server and all of its components?");
        let title = HSTRING::from("I Connect Uninstall");
        if MessageBoxW(
            HWND::default(),
            PCWSTR(msg.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNO | MB_ICONQUESTION,
        ) != IDYES
        {
            return;
        }

        // 1. Gracefully terminate running I Connect Server instance
        let cls = HSTRING::from("IConnectServerClass");
        if let Ok(h_server) = FindWindowW(PCWSTR(cls.as_ptr()), PCWSTR::null()) {
            if !h_server.is_invalid() {
                SendMessageW(h_server, WM_COMMAND, WPARAM(2004), LPARAM(0));
                thread::sleep(Duration::from_millis(500));
            }
        }

        // 2. Remove Startup Registry Key
        let run_subkey = HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Run");
        let run_val = HSTRING::from("IConnectServer");
        let mut h_run_key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(run_subkey.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut h_run_key,
        ) == ERROR_SUCCESS
        {
            let _ = RegDeleteValueW(h_run_key, PCWSTR(run_val.as_ptr()));
            let _ = RegCloseKey(h_run_key);
        }

        // 3. Remove Uninstall Registry Key
        let uninst_subkey = HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\IConnect");
        let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(uninst_subkey.as_ptr()));
    }

    // 4. Remove Desktop & Start Menu shortcuts
    if let Some(desktop) = get_shell_folder(CSIDL_DESKTOPDIRECTORY as i32) {
        let _ = fs::remove_file(desktop.join("I Connect Server.lnk"));
    }
    if let Some(programs) = get_shell_folder(CSIDL_PROGRAMS as i32) {
        let _ = fs::remove_file(programs.join("I Connect Server.lnk"));
    }

    // 5. Remove Windows Firewall rules silently
    for rule in ["I Connect Server", "I Connect Server Ports", "I Connect UDP"] {
        let _ = Command::new("netsh")
            .args(["advfirewall", "firewall", "delete", "rule", &format!("name={}", rule)])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }

    // 6. Remove installed files and schedule directory cleanup
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(dir) = exe_path.parent() {
            let _ = fs::remove_file(dir.join("I_Connect_Server.exe"));
            let _ = fs::remove_file(dir.join("pc_icon.ico"));
            let _ = fs::remove_file(dir.join("paired_devices.txt"));

            let dir_str = dir.to_string_lossy().into_owned();
            let exe_str = exe_path.to_string_lossy().into_owned();
            let cleanup_cmd = format!(
                "ping 127.0.0.1 -n 2 > nul & del /f /q \"{}\" & rd /s /q \"{}\"",
                exe_str, dir_str
            );
            let _ = Command::new("cmd.exe")
                .args(["/c", &cleanup_cmd])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
        }
    }

    unsafe {
        let done_msg = HSTRING::from("I Connect Server was successfully removed from your computer.");
        let done_title = HSTRING::from("Uninstall Complete");
        MessageBoxW(
            HWND::default(),
            PCWSTR(done_msg.as_ptr()),
            PCWSTR(done_title.as_ptr()),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}
