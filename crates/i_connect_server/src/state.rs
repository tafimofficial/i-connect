use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_SZ,
};

pub const CONFIG_FILE: &str = "paired_devices.txt";
pub const AUTOSTART_REG_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const AUTOSTART_VALUE_NAME: &str = "IConnectServer";

#[derive(Debug, Clone)]
pub struct TransferProgress {
    pub is_transferring: bool,
    pub is_sending: bool,
    pub current_file_name: String,
    pub current_file_size: u64,
    pub current_file_transferred: u64,
    pub total_files_in_batch: u32,
    pub current_file_index: u32,
    pub current_speed_mbps: f32,
}

impl Default for TransferProgress {
    fn default() -> Self {
        Self {
            is_transferring: false,
            is_sending: false,
            current_file_name: String::new(),
            current_file_size: 0,
            current_file_transferred: 0,
            total_files_in_batch: 0,
            current_file_index: 0,
            current_speed_mbps: 0.0,
        }
    }
}

pub struct AppState {
    pub hwnd_raw: isize,
    pub local_ip: String,
    pub gateway_ip: String,
    pub is_hidden_in_tray: bool,

    // Connected client state
    pub is_connected: bool,
    pub bind_error: Option<String>,
    pub client_name: String,
    pub client_id: String,
    pub client_ip: String,
    pub client_file_stream: Option<TcpStream>,
    pub client_file_conn_id: u64,
    pub client_file_workers: Vec<(u64, TcpStream)>,

    // Live transfer stats
    pub transfer: TransferProgress,

    // Total stats
    pub total_packets_received: u64,
    pub paired_count: usize,

    // UI hover state (-1 = none, 0 = Open Downloads, 1 = Windows Boot, 2 = Reset Paired, 3 = Copy IP)
    pub hovered_btn: i32,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            hwnd_raw: 0,
            local_ip: "127.0.0.1".to_string(),
            gateway_ip: String::new(),
            is_hidden_in_tray: false,
            is_connected: false,
            bind_error: None,
            client_name: "None".to_string(),
            client_id: String::new(),
            client_ip: String::new(),
            client_file_stream: None,
            client_file_conn_id: 0,
            client_file_workers: Vec::new(),
            transfer: TransferProgress::default(),
            total_packets_received: 0,
            paired_count: count_paired_devices(),
            hovered_btn: -1,
        }
    }
}

pub type SharedState = Arc<Mutex<AppState>>;
pub type RunningFlag = Arc<AtomicBool>;

pub fn hwnd_from_raw(raw: isize) -> HWND {
    HWND(raw as *mut core::ffi::c_void)
}

pub fn get_save_directory() -> PathBuf {
    let base = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|p| p.join("Downloads").join("I_Connect_Transfers"))
        .unwrap_or_else(|| PathBuf::from(r"C:\I_Connect_Transfers"));
    let _ = fs::create_dir_all(&base);
    base
}

fn candidate_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join(CONFIG_FILE));
        }
    }
    paths.push(PathBuf::from(CONFIG_FILE));
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        paths.push(PathBuf::from(local).join("Programs").join("IConnect").join(CONFIG_FILE));
    }
    paths
}

fn load_paired_ids_from(path: &Path, set: &mut HashSet<String>) {
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let id = line.split('|').next().unwrap_or("").trim();
            if !id.is_empty() {
                set.insert(id.to_string());
            }
        }
    }
}

pub fn count_paired_devices() -> usize {
    let mut ids = HashSet::new();
    for path in candidate_config_paths() {
        load_paired_ids_from(&path, &mut ids);
    }
    ids.len()
}

pub fn is_device_paired(device_id: &str) -> bool {
    if device_id.trim().is_empty() {
        return false;
    }
    for path in candidate_config_paths() {
        if let Ok(content) = fs::read_to_string(&path) {
            for line in content.lines() {
                let id = line.split('|').next().unwrap_or("").trim();
                if id == device_id {
                    return true;
                }
            }
        }
    }
    false
}

pub fn save_device_to_whitelist(device_id: &str, device_name: &str) {
    if device_id.trim().is_empty() || is_device_paired(device_id) {
        return;
    }
    let clean_name = if device_name.trim().is_empty() {
        "Unknown Phone"
    } else {
        device_name.trim()
    };
    let line = format!("{}|{}\n", device_id.trim(), clean_name);

    for path in candidate_config_paths() {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = fs::create_dir_all(parent);
            }
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
            if f.write_all(line.as_bytes()).is_ok() {
                let _ = f.flush();
                return;
            }
        }
    }
}

pub fn clear_paired_devices() {
    for path in candidate_config_paths() {
        let _ = fs::remove_file(path);
    }
}

pub fn is_autostart_enabled() -> bool {
    unsafe {
        let subkey = HSTRING::from(AUTOSTART_REG_KEY);
        let value_name = HSTRING::from(AUTOSTART_VALUE_NAME);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_READ,
            &mut hkey,
        ) == ERROR_SUCCESS
        {
            let mut size: u32 = 0;
            let res = RegQueryValueExW(
                hkey,
                PCWSTR(value_name.as_ptr()),
                None,
                None,
                None,
                Some(&mut size),
            );
            let _ = RegCloseKey(hkey);
            return res == ERROR_SUCCESS;
        }
        false
    }
}

pub fn set_autostart(enable: bool) {
    unsafe {
        let subkey = HSTRING::from(AUTOSTART_REG_KEY);
        let value_name = HSTRING::from(AUTOSTART_VALUE_NAME);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        ) == ERROR_SUCCESS
        {
            if enable {
                if let Ok(exe) = std::env::current_exe() {
                    let cmd = format!("\"{}\" --minimized", exe.display());
                    let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
                    let byte_slice = std::slice::from_raw_parts(
                        wide.as_ptr() as *const u8,
                        wide.len() * std::mem::size_of::<u16>(),
                    );
                    let _ = RegSetValueExW(
                        hkey,
                        PCWSTR(value_name.as_ptr()),
                        0,
                        REG_SZ,
                        Some(byte_slice),
                    );
                }
            } else {
                let _ = RegDeleteValueW(hkey, PCWSTR(value_name.as_ptr()));
            }
            let _ = RegCloseKey(hkey);
        }
    }
}
