#![windows_subsystem = "windows"]

mod input;
mod network;
mod state;
mod ui;

use crate::network::{
    get_local_network_info, send_dropped_paths_to_phone, spawn_network_threads, WM_APP_RESTORE,
    WM_APP_TRAY_CONNECTED, WM_APP_TRAY_DISCONNECTED, WM_APP_TRAY_FILE_DONE,
};
use crate::state::{
    clear_paired_devices, count_paired_devices, get_save_directory, is_autostart_enabled,
    set_autostart, AppState, RunningFlag, SharedState,
};
use crate::ui::{hit_test_button, paint_main_window};
use std::mem::size_of;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, BOOL, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT,
    RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, PAINTSTRUCT};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{
    DragAcceptFiles, DragFinish, DragQueryFileW, ShellExecuteW, Shell_NotifyIconW, HDROP,
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW, FindWindowW,
    GetClientRect, GetCursorPos, GetMessageW, GetSystemMetrics, InsertMenuW, LoadCursorW,
    LoadIconW, MessageBoxW, PostQuitMessage, RegisterClassExW, SetCursor, SetForegroundWindow,
    SetTimer, ShowWindow, TrackPopupMenu, TranslateMessage, CS_HREDRAW, CS_VREDRAW, HICON,
    IDC_ARROW, IDC_HAND, IDI_APPLICATION, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK,
    MB_YESNO, MF_BYPOSITION, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, SM_CXSCREEN,
    SM_CYSCREEN, SW_HIDE, SW_RESTORE, SW_SHOW, SW_SHOWNORMAL, TPM_RIGHTBUTTON, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DROPFILES, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONUP, WM_TIMER, WM_USER, WNDCLASSEXW,
    WS_CAPTION, WS_EX_ACCEPTFILES, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
};

const APP_TITLE: &str = "I Connect Server [Turbo Link]";
const CLASS_NAME: &str = "IConnectServerClass";
const WM_TRAYICON: u32 = WM_USER + 101;
const CF_UNICODETEXT: u32 = 13;

const ID_TRAY_OPEN: usize = 2001;
const ID_TRAY_AUTOSTART: usize = 2002;
const ID_TRAY_RESET: usize = 2003;
const ID_TRAY_EXIT: usize = 2004;

struct RuntimeCtx {
    state: SharedState,
    running: RunningFlag,
    h_icon: isize,
    copied_until: Mutex<Option<Instant>>,
}

static CTX: OnceLock<RuntimeCtx> = OnceLock::new();

fn ctx() -> &'static RuntimeCtx {
    CTX.get().expect("RuntimeCtx initialized")
}

fn fill_wbuf(dst: &mut [u16], s: &str) {
    dst.fill(0);
    let max_len = dst.len().saturating_sub(1);
    for (i, ch) in s.encode_utf16().take(max_len).enumerate() {
        dst[i] = ch;
    }
}

unsafe fn init_tray_icon(hwnd: HWND, h_icon: HICON) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_INFO,
        uCallbackMessage: WM_TRAYICON,
        hIcon: h_icon,
        ..Default::default()
    };
    fill_wbuf(&mut nid.szTip, "I Connect Server [Running in Background]");
    let _ = Shell_NotifyIconW(NIM_ADD, &nid);
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
}

unsafe fn update_tray_info(hwnd: HWND, h_icon: HICON, tip: &str, balloon: Option<(&str, &str)>) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | if balloon.is_some() { NIF_INFO } else { Default::default() },
        uCallbackMessage: WM_TRAYICON,
        hIcon: h_icon,
        dwInfoFlags: NIIF_INFO,
        ..Default::default()
    };
    fill_wbuf(&mut nid.szTip, tip);
    if let Some((title, msg)) = balloon {
        fill_wbuf(&mut nid.szInfoTitle, title);
        fill_wbuf(&mut nid.szInfo, msg);
    }
    let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
}

unsafe fn minimize_to_tray(hwnd: HWND) {
    let _ = ShowWindow(hwnd, SW_HIDE);
    if let Ok(mut s) = ctx().state.lock() {
        s.is_hidden_in_tray = true;
    }
    let h_icon = HICON(ctx().h_icon as *mut core::ffi::c_void);
    update_tray_info(
        hwnd,
        h_icon,
        "I Connect Server [Running in Background]",
        Some((
            "I Connect Server",
            "Running in the background. Click the tray icon to open dashboard.",
        )),
    );
}

unsafe fn restore_from_tray(hwnd: HWND) {
    let _ = ShowWindow(hwnd, SW_SHOW);
    let _ = ShowWindow(hwnd, SW_RESTORE);
    let _ = SetForegroundWindow(hwnd);
    if let Ok(mut s) = ctx().state.lock() {
        s.is_hidden_in_tray = false;
    }
}

unsafe fn copy_text_to_clipboard(hwnd: HWND, text: &str) {
    if OpenClipboard(hwnd).is_err() {
        return;
    }
    let _ = EmptyClipboard();
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = wide.len() * size_of::<u16>();
    if let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, bytes) {
        let ptr = GlobalLock(hmem);
        if !ptr.is_null() {
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr as *mut u16, wide.len());
            let _ = GlobalUnlock(hmem);
            let _ = SetClipboardData(CF_UNICODETEXT, windows::Win32::Foundation::HANDLE(hmem.0));
        }
    }
    let _ = CloseClipboard();
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            DragAcceptFiles(hwnd, true);
            SetTimer(hwnd, 1, 350, None);
            let h_icon = HICON(ctx().h_icon as *mut core::ffi::c_void);
            init_tray_icon(hwnd, h_icon);
            LRESULT(0)
        }
        WM_DROPFILES => {
            let hdrop = HDROP(wparam.0 as *mut core::ffi::c_void);
            let count = DragQueryFileW(hdrop, 0xFFFF_FFFF, None);
            let mut paths = Vec::with_capacity(count as usize);
            let mut buf = vec![0u16; 1024];
            for i in 0..count {
                let len = DragQueryFileW(hdrop, i, Some(&mut buf));
                if len > 0 {
                    let s = String::from_utf16_lossy(&buf[..len as usize]);
                    paths.push(PathBuf::from(s));
                }
            }
            DragFinish(hdrop);
            if !paths.is_empty() {
                send_dropped_paths_to_phone(paths, ctx().state.clone());
            }
            LRESULT(0)
        }
        WM_APP_RESTORE => {
            restore_from_tray(hwnd);
            LRESULT(0)
        }
        WM_APP_TRAY_CONNECTED => {
            let name = ctx()
                .state
                .lock()
                .map(|s| s.client_name.clone())
                .unwrap_or_else(|_| "Phone".into());
            let tip = format!("I Connect: Connected to {}", name);
            let h_icon = HICON(ctx().h_icon as *mut core::ffi::c_void);
            update_tray_info(hwnd, h_icon, &tip, Some(("Phone Connected", &name)));
            LRESULT(0)
        }
        WM_APP_TRAY_DISCONNECTED => {
            let h_icon = HICON(ctx().h_icon as *mut core::ffi::c_void);
            update_tray_info(hwnd, h_icon, "I Connect Server [Running in Background]", None);
            LRESULT(0)
        }
        WM_APP_TRAY_FILE_DONE => {
            let h_icon = HICON(ctx().h_icon as *mut core::ffi::c_void);
            update_tray_info(
                hwnd,
                h_icon,
                "I Connect Server [Running in Background]",
                Some((
                    "File Received",
                    "Transfer complete! Saved to Downloads\\I_Connect_Transfers",
                )),
            );
            LRESULT(0)
        }
        WM_TRAYICON => {
            let ev = (lparam.0 & 0xFFFF) as u32;
            if ev == WM_LBUTTONDBLCLK || ev == WM_LBUTTONUP {
                restore_from_tray(hwnd);
            } else if ev == WM_RBUTTONUP {
                let mut pt = POINT::default();
                let _ = GetCursorPos(&mut pt);
                if let Ok(hmenu) = CreatePopupMenu() {
                    let s_open = HSTRING::from("Open Dashboard");
                    let s_boot = HSTRING::from("Start with Windows Boot");
                    let s_reset = HSTRING::from("Reset Paired Phones");
                    let s_exit = HSTRING::from("Exit Server Completely");

                    let _ = InsertMenuW(hmenu, 0, MF_BYPOSITION | MF_STRING, ID_TRAY_OPEN, PCWSTR(s_open.as_ptr()));
                    let boot_flag = if is_autostart_enabled() { MF_CHECKED } else { MF_UNCHECKED };
                    let _ = InsertMenuW(hmenu, 1, MF_BYPOSITION | boot_flag | MF_STRING, ID_TRAY_AUTOSTART, PCWSTR(s_boot.as_ptr()));
                    let _ = InsertMenuW(hmenu, 2, MF_BYPOSITION | MF_STRING, ID_TRAY_RESET, PCWSTR(s_reset.as_ptr()));
                    let _ = InsertMenuW(hmenu, 3, MF_BYPOSITION | MF_SEPARATOR, 0, PCWSTR::null());
                    let _ = InsertMenuW(hmenu, 4, MF_BYPOSITION | MF_STRING, ID_TRAY_EXIT, PCWSTR(s_exit.as_ptr()));

                    let _ = SetForegroundWindow(hwnd);
                    let _ = TrackPopupMenu(hmenu, TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, None);
                    let _ = DestroyMenu(hmenu);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd = wparam.0 & 0xFFFF;
            if cmd == ID_TRAY_OPEN {
                restore_from_tray(hwnd);
            } else if cmd == ID_TRAY_AUTOSTART {
                set_autostart(!is_autostart_enabled());
                let _ = InvalidateRect(hwnd, None, false);
            } else if cmd == ID_TRAY_RESET {
                clear_paired_devices();
                if let Ok(mut s) = ctx().state.lock() {
                    s.paired_count = 0;
                }
                let _ = InvalidateRect(hwnd, None, false);
                let msg = HSTRING::from("Paired device whitelist reset successfully!");
                let title = HSTRING::from("I Connect - Reset");
                MessageBoxW(hwnd, PCWSTR(msg.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONINFORMATION);
            } else if cmd == ID_TRAY_EXIT {
                ctx().running.store(false, Ordering::SeqCst);
                remove_tray_icon(hwnd);
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let hovered = hit_test_button(rc.right - rc.left, x, y);
            let cursor_id = if hovered >= 0 { IDC_HAND } else { IDC_ARROW };
            if let Ok(cur) = LoadCursorW(None, cursor_id) {
                SetCursor(cur);
            }
            let changed = ctx()
                .state
                .lock()
                .map(|mut s| {
                    if s.hovered_btn != hovered {
                        s.hovered_btn = hovered;
                        true
                    } else {
                        false
                    }
                })
                .unwrap_or(false);
            if changed {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            match hit_test_button(rc.right - rc.left, x, y) {
                0 => {
                    let save_dir = get_save_directory();
                    let open = HSTRING::from("open");
                    let path = HSTRING::from(save_dir.to_string_lossy().as_ref());
                    ShellExecuteW(
                        HWND::default(),
                        PCWSTR(open.as_ptr()),
                        PCWSTR(path.as_ptr()),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOWNORMAL,
                    );
                }
                1 => {
                    set_autostart(!is_autostart_enabled());
                    let _ = InvalidateRect(hwnd, None, false);
                }
                2 => {
                    let msg = HSTRING::from("Clear all saved paired phones?\nYou will be prompted again next time they connect.");
                    let title = HSTRING::from("Reset Pairing Whitelist");
                    if MessageBoxW(hwnd, PCWSTR(msg.as_ptr()), PCWSTR(title.as_ptr()), MB_YESNO | MB_ICONQUESTION) == IDYES {
                        clear_paired_devices();
                        if let Ok(mut s) = ctx().state.lock() {
                            s.paired_count = 0;
                        }
                        let _ = InvalidateRect(hwnd, None, false);
                        let done = HSTRING::from("Paired whitelist cleared successfully!");
                        let t2 = HSTRING::from("Success");
                        MessageBoxW(hwnd, PCWSTR(done.as_ptr()), PCWSTR(t2.as_ptr()), MB_OK | MB_ICONINFORMATION);
                    }
                }
                3 => {
                    let ip = ctx().state.lock().map(|s| s.local_ip.clone()).unwrap_or_default();
                    if !ip.is_empty() {
                        copy_text_to_clipboard(hwnd, &ip);
                        if let Ok(mut c) = ctx().copied_until.lock() {
                            *c = Some(Instant::now() + std::time::Duration::from_secs(2));
                        }
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            minimize_to_tray(hwnd);
            LRESULT(0)
        }
        WM_TIMER => {
            let need_paint = {
                let transferring = ctx().state.lock().map(|s| s.transfer.is_transferring).unwrap_or(false);
                let mut clear_flash = false;
                if let Ok(mut c) = ctx().copied_until.lock() {
                    if let Some(until) = *c {
                        if Instant::now() >= until {
                            *c = None;
                            clear_flash = true;
                        }
                    }
                }
                transferring || clear_flash
            };
            if need_paint {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let flash = ctx()
                .copied_until
                .lock()
                .ok()
                .and_then(|c| *c)
                .map(|until| Instant::now() < until)
                .unwrap_or(false);
            if let Ok(s) = ctx().state.lock() {
                paint_main_window(hwnd, hdc, &s, flash);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            ctx().running.store(false, Ordering::SeqCst);
            remove_tray_icon(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    // Ensure working directory is always executable folder (for Windows Startup launch)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let _ = std::env::set_current_dir(dir);
        }
    }

    unsafe {
        // Single-instance check
        let mutex_name = HSTRING::from("Global\\IConnectServerMutex");
        let _h_mutex = CreateMutexW(None, true, PCWSTR(mutex_name.as_ptr()));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let cls = HSTRING::from(CLASS_NAME);
            let title = HSTRING::from(APP_TITLE);
            if let Ok(existing) = FindWindowW(PCWSTR(cls.as_ptr()), PCWSTR(title.as_ptr())) {
                if !existing.is_invalid() {
                    let _ = ShowWindow(existing, SW_SHOW);
                    let _ = ShowWindow(existing, SW_RESTORE);
                    let _ = SetForegroundWindow(existing);
                }
            }
            return;
        }

        let h_inst_mod = GetModuleHandleW(None).unwrap_or_default();
        let h_inst = HINSTANCE(h_inst_mod.0);
        let h_icon = LoadIconW(h_inst, PCWSTR(101usize as *const u16))
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default();

        let (local_ip, gateway_ip) = get_local_network_info();
        let mut init_state = AppState::default();
        init_state.local_ip = local_ip;
        init_state.gateway_ip = gateway_ip;
        init_state.paired_count = count_paired_devices();

        let shared_state = Arc::new(Mutex::new(init_state));
        let running = Arc::new(AtomicBool::new(true));

        let _ = CTX.set(RuntimeCtx {
            state: shared_state.clone(),
            running: running.clone(),
            h_icon: h_icon.0 as isize,
            copied_until: Mutex::new(None),
        });

        let cls_w = HSTRING::from(CLASS_NAME);
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: h_inst,
            hIcon: h_icon,
            hCursor: cursor,
            lpszClassName: PCWSTR(cls_w.as_ptr()),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return;
        }

        let win_w = 690;
        let win_h = 505;
        let scr_w = GetSystemMetrics(SM_CXSCREEN);
        let scr_h = GetSystemMetrics(SM_CYSCREEN);
        let win_x = (scr_w - win_w) / 2;
        let win_y = (scr_h - win_h) / 2;

        let title_w = HSTRING::from(APP_TITLE);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_ACCEPTFILES,
            PCWSTR(cls_w.as_ptr()),
            PCWSTR(title_w.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            win_x,
            win_y,
            win_w,
            win_h,
            None,
            None,
            h_inst,
            None,
        ) else {
            return;
        };

        if let Ok(mut s) = shared_state.lock() {
            s.hwnd_raw = hwnd.0 as isize;
        }

        // Enable dark immersive titlebar on Windows 10/11
        let dark_mode = BOOL(1);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_mode as *const BOOL as *const _,
            size_of::<BOOL>() as u32,
        );

        let start_minimized = std::env::args().any(|a| a == "--minimized" || a == "--autostart");
        if start_minimized {
            if let Ok(mut s) = shared_state.lock() {
                s.is_hidden_in_tray = true;
            }
            let _ = ShowWindow(hwnd, SW_HIDE);
        } else {
            let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        }

        spawn_network_threads(shared_state, running);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
