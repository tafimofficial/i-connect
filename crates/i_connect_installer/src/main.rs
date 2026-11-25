#![windows_subsystem = "windows"]

use std::fs;
use std::mem::size_of;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::{BOOL, COLORREF, ERROR_SUCCESS, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject,
    DrawTextW, EndPaint, InvalidateRect, SelectObject, SetBkMode, SetTextColor, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
    DEFAULT_PITCH, DIB_RGB_COLORS, DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT,
    DT_SINGLELINE, DT_NOPREFIX, DT_VCENTER, FF_DONTCARE, FW_BOLD, FW_NORMAL, FW_SEMIBOLD, HDC, HGDIOBJ,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
    KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};
use windows::Win32::UI::Shell::{
    IShellLinkW, SHGetFolderPathW, ShellExecuteW, ShellLink, CSIDL_DESKTOPDIRECTORY,
    CSIDL_PROGRAMS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
    GetSystemMetrics, LoadCursorW, LoadIconW, PostMessageW, PostQuitMessage, RegisterClassExW,
    SetCursor, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, IDC_ARROW, IDC_HAND,
    IDI_APPLICATION, MSG, SM_CXSCREEN, SM_CYSCREEN, SW_SHOWNORMAL, WM_CREATE, WM_DESTROY,
    WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_PAINT, WM_USER, WNDCLASSEXW, WS_CAPTION, WS_MINIMIZEBOX,
    WS_OVERLAPPED, WS_SYSMENU,
};

const PAYLOAD_SERVER_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/I_Connect_Server.exe"));
const PAYLOAD_UNINSTALL_EXE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/uninstall.exe"));
const PAYLOAD_ICON_ICO: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pc_icon.ico"));

const CREATE_NO_WINDOW: u32 = 0x08000000;
const WM_APP_REPAINT: u32 = WM_USER + 301;

// AMOLED Black + Electric Lime Theme
const COL_BG: u32 = 0x00_000000;
const COL_CARD: u32 = 0x00_0A0D08;
const COL_CARD_BORDER: u32 = 0x00_1F2E14;
const COL_LIME: u32 = 0x00_B6FF00;
const COL_LIME_DIM: u32 = 0x00_84CC16;
const COL_TEXT_MAIN: u32 = 0x00_F4FCE8;
const COL_TEXT_MUTED: u32 = 0x00_889A78;
const COL_PROGRESS_BG: u32 = 0x00_151E0F;

#[derive(Clone)]
struct InstallerState {
    hwnd_raw: isize,
    step: i32, // 0 = Welcome/Options, 2 = Installing, 3 = Finished
    install_dir: PathBuf,
    opt_autostart: bool,
    opt_desktop_shortcut: bool,
    opt_start_menu: bool,
    opt_launch_now: bool,
    progress_pct: i32,
    status_text: String,
    hovered_btn: i32,
}

static STATE: OnceLock<Mutex<InstallerState>> = OnceLock::new();

fn state() -> &'static Mutex<InstallerState> {
    STATE.get().expect("InstallerState initialized")
}

#[inline]
fn rgb_to_cref(rgb: u32) -> COLORREF {
    let r = (rgb >> 16) & 0xFF;
    let g = (rgb >> 8) & 0xFF;
    let b = rgb & 0xFF;
    COLORREF((b << 16) | (g << 8) | r)
}

#[inline]
fn blend_px(bg: u32, fg: u32, alpha: f32) -> u32 {
    let a = alpha.clamp(0.0, 1.0);
    let inv = 1.0 - a;
    let r = ((((bg >> 16) & 0xFF) as f32) * inv + (((fg >> 16) & 0xFF) as f32) * a).round() as u32;
    let g = ((((bg >> 8) & 0xFF) as f32) * inv + (((fg >> 8) & 0xFF) as f32) * a).round() as u32;
    let b = (((bg & 0xFF) as f32) * inv + ((fg & 0xFF) as f32) * a).round() as u32;
    (r << 16) | (g << 8) | b
}

#[inline]
fn sd_rounded_box(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let dx = (px - cx).abs() - hx + r;
    let dy = (py - cy).abs() - hy + r;
    (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() + dx.max(dy).min(0.0) - r
}

fn draw_rounded_box(
    pixels: &mut [u32],
    w_buf: i32,
    h_buf: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: f32,
    fill: u32,
    border: Option<(u32, f32)>,
) {
    let x0 = (x - 2).max(0);
    let y0 = (y - 2).max(0);
    let x1 = (x + w + 2).min(w_buf - 1);
    let y1 = (y + h + 2).min(h_buf - 1);
    let cx = x as f32 + w as f32 * 0.5;
    let cy = y as f32 + h as f32 * 0.5;
    let hx = w as f32 * 0.5;
    let hy = h as f32 * 0.5;
    let rr = r.min(hx).min(hy);

    for py in y0..=y1 {
        for px in x0..=x1 {
            let d = sd_rounded_box(px as f32 + 0.5, py as f32 + 0.5, cx, cy, hx, hy, rr);
            let idx = (py * w_buf + px) as usize;
            if d < 1.0 {
                let fa = (0.5 - d).clamp(0.0, 1.0);
                pixels[idx] = blend_px(pixels[idx], fill, fa);
                if let Some((bc, bw)) = border {
                    let bd = (d + bw * 0.5).abs() - bw * 0.5;
                    let ba = (0.5 - bd).clamp(0.0, 1.0);
                    pixels[idx] = blend_px(pixels[idx], bc, ba);
                }
            } else if let Some((bc, bw)) = border {
                let bd = (d + bw * 0.5).abs() - bw * 0.5;
                if bd < 0.75 {
                    let ba = (0.5 - bd).clamp(0.0, 1.0);
                    pixels[idx] = blend_px(pixels[idx], bc, ba);
                }
            }
        }
    }
}

unsafe fn make_font(height: i32, weight: i32) -> HGDIOBJ {
    let face = HSTRING::from("Segoe UI");
    let f = CreateFontW(
        height,
        0,
        0,
        0,
        weight,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        CLEARTYPE_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
        PCWSTR(face.as_ptr()),
    );
    HGDIOBJ(f.0)
}

unsafe fn draw_text_rect(
    hdc: HDC,
    text: &str,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u32,
    flags: DRAW_TEXT_FORMAT,
) {
    SetTextColor(hdc, rgb_to_cref(color));
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut rc = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    DrawTextW(hdc, &mut wide, &mut rc, flags | DT_NOPREFIX);
}

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

unsafe fn create_shell_link(
    target_exe: &Path,
    shortcut_path: &Path,
    icon_path: &Path,
    description: &str,
) {
    let Ok(psl): windows::core::Result<IShellLinkW> =
        CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
    else {
        return;
    };

    let tgt = HSTRING::from(target_exe.to_string_lossy().as_ref());
    let _ = psl.SetPath(PCWSTR(tgt.as_ptr()));
    if let Some(dir) = target_exe.parent() {
        let wd = HSTRING::from(dir.to_string_lossy().as_ref());
        let _ = psl.SetWorkingDirectory(PCWSTR(wd.as_ptr()));
    }
    let desc = HSTRING::from(description);
    let _ = psl.SetDescription(PCWSTR(desc.as_ptr()));
    let ico = HSTRING::from(icon_path.to_string_lossy().as_ref());
    let _ = psl.SetIconLocation(PCWSTR(ico.as_ptr()), 0);

    if let Ok(ppf) = psl.cast::<IPersistFile>() {
        let lnk = HSTRING::from(shortcut_path.to_string_lossy().as_ref());
        let _ = ppf.Save(PCWSTR(lnk.as_ptr()), true);
    }
}

fn run_silent(program: &str, args: &[&str]) {
    let _ = Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

unsafe fn write_reg_sz(hkey: HKEY, name: &str, val: &str) {
    let w_name = HSTRING::from(name);
    let wide: Vec<u16> = val.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
    let _ = RegSetValueExW(hkey, PCWSTR(w_name.as_ptr()), 0, REG_SZ, Some(bytes));
}

fn update_progress(pct: i32, status: &str, step: Option<i32>) {
    let raw = {
        let mut s = state().lock().unwrap();
        s.progress_pct = pct;
        s.status_text = status.to_string();
        if let Some(st) = step {
            s.step = st;
        }
        s.hwnd_raw
    };
    if raw != 0 {
        unsafe {
            let _ = PostMessageW(
                HWND(raw as *mut core::ffi::c_void),
                WM_APP_REPAINT,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

fn start_installation_worker() {
    let snapshot = state().lock().unwrap().clone();
    update_progress(10, "Creating installation directory...", Some(2));

    thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }

        thread::sleep(Duration::from_millis(180));
        let _ = fs::create_dir_all(&snapshot.install_dir);

        // 2. Extract Server
        update_progress(30, "Extracting I_Connect_Server.exe...", None);
        thread::sleep(Duration::from_millis(180));
        let exe_dest = snapshot.install_dir.join("I_Connect_Server.exe");
        let _ = fs::write(&exe_dest, PAYLOAD_SERVER_EXE);

        // 3. Extract Icon & Uninstaller
        update_progress(50, "Installing application icons & uninstaller...", None);
        thread::sleep(Duration::from_millis(180));
        let ico_dest = snapshot.install_dir.join("pc_icon.ico");
        let _ = fs::write(&ico_dest, PAYLOAD_ICON_ICO);
        let uninst_dest = snapshot.install_dir.join("uninstall.exe");
        let _ = fs::write(&uninst_dest, PAYLOAD_UNINSTALL_EXE);

        // 4. Configure Startup & Uninstall Registry
        update_progress(70, "Configuring Windows Startup & Registry...", None);
        thread::sleep(Duration::from_millis(180));
        unsafe {
            if snapshot.opt_autostart {
                let run_key = HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Run");
                let mut hkey = HKEY::default();
                if RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    PCWSTR(run_key.as_ptr()),
                    0,
                    KEY_SET_VALUE,
                    &mut hkey,
                ) == ERROR_SUCCESS
                {
                    let cmd = format!("\"{}\" --minimized", exe_dest.display());
                    write_reg_sz(hkey, "IConnectServer", &cmd);
                    let _ = RegCloseKey(hkey);
                }
            }

            let uninst_key =
                HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\IConnect");
            let mut h_uninst = HKEY::default();
            if RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(uninst_key.as_ptr()),
                0,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut h_uninst,
                None,
            ) == ERROR_SUCCESS
            {
                write_reg_sz(h_uninst, "DisplayName", "I Connect Server");
                write_reg_sz(h_uninst, "DisplayIcon", &ico_dest.to_string_lossy());
                write_reg_sz(h_uninst, "UninstallString", &uninst_dest.to_string_lossy());
                write_reg_sz(h_uninst, "Publisher", "I Connect");
                write_reg_sz(h_uninst, "DisplayVersion", "1.0.0");
                let _ = RegCloseKey(h_uninst);
            }
        }

        // 5. Shortcuts & Firewall Rules
        update_progress(90, "Creating shortcuts & Windows Firewall rules...", None);
        thread::sleep(Duration::from_millis(180));
        unsafe {
            if snapshot.opt_desktop_shortcut {
                if let Some(desktop) = get_shell_folder(CSIDL_DESKTOPDIRECTORY as i32) {
                    create_shell_link(
                        &exe_dest,
                        &desktop.join("I Connect Server.lnk"),
                        &ico_dest,
                        "I Connect Server",
                    );
                }
            }
            if snapshot.opt_start_menu {
                if let Some(programs) = get_shell_folder(CSIDL_PROGRAMS as i32) {
                    create_shell_link(
                        &exe_dest,
                        &programs.join("I Connect Server.lnk"),
                        &ico_dest,
                        "I Connect Server",
                    );
                }
            }
        }

        let prog_arg = format!("program={}", exe_dest.display());
        run_silent(
            "netsh",
            &[
                "advfirewall",
                "firewall",
                "add",
                "rule",
                "name=I Connect Server",
                "dir=in",
                "action=allow",
                &prog_arg,
                "enable=yes",
                "profile=any",
            ],
        );
        run_silent(
            "netsh",
            &[
                "advfirewall",
                "firewall",
                "add",
                "rule",
                "name=I Connect Server Ports",
                "dir=in",
                "action=allow",
                "protocol=TCP",
                "localport=52200,52201",
                "profile=any",
            ],
        );
        run_silent(
            "netsh",
            &[
                "advfirewall",
                "firewall",
                "add",
                "rule",
                "name=I Connect UDP",
                "dir=in",
                "action=allow",
                "protocol=UDP",
                "localport=52199",
                "profile=any",
            ],
        );

        update_progress(100, "Installation completed successfully!", Some(3));
        unsafe {
            CoUninitialize();
        }
    });
}

fn hit_test(width: i32, x: i32, y: i32, step: i32) -> i32 {
    if step == 0 {
        if x >= 45 && x <= 480 && y >= 166 && y <= 194 {
            return 10; // autostart
        }
        if x >= 45 && x <= 420 && y >= 202 && y <= 230 {
            return 11; // desktop shortcut
        }
        if x >= 45 && x <= 420 && y >= 238 && y <= 266 {
            return 12; // start menu
        }
        if x >= width - 190 && x <= width - 24 && y >= 344 && y <= 386 {
            return 13; // Install Now
        }
    } else if step == 3 {
        if x >= 45 && x <= 420 && y >= 244 && y <= 274 {
            return 20; // launch now
        }
        if x >= width - 166 && x <= width - 24 && y >= 344 && y <= 386 {
            return 21; // Finish
        }
    }
    -1
}

unsafe fn paint_installer(hwnd: HWND, hdc: HDC, st: &InstallerState) {
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let width = (rc.right - rc.left).max(1);
    let height = (rc.bottom - rc.top).max(1);

    let mem_dc = CreateCompatibleDC(hdc);
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let Ok(dib) = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
        let _ = DeleteDC(mem_dc);
        return;
    };
    if bits.is_null() {
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(mem_dc);
        return;
    }
    let old_bmp = SelectObject(mem_dc, HGDIOBJ(dib.0));

    {
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (width * height) as usize);
        pixels.fill(COL_BG);

        draw_rounded_box(
            pixels,
            width,
            height,
            24,
            82,
            width - 48,
            246,
            14.0,
            COL_CARD,
            Some((COL_CARD_BORDER, 1.2)),
        );

        if st.step == 0 {
            let btn_col = if st.hovered_btn == 13 { 0x00_CCFF44 } else { COL_LIME };
            draw_rounded_box(
                pixels,
                width,
                height,
                width - 190,
                344,
                166,
                42,
                10.0,
                btn_col,
                None,
            );
        } else if st.step == 2 {
            draw_rounded_box(
                pixels,
                width,
                height,
                45,
                192,
                width - 90,
                20,
                9.0,
                COL_PROGRESS_BG,
                Some((COL_CARD_BORDER, 1.0)),
            );
            let fw = (((width - 90) as f32) * (st.progress_pct as f32 / 100.0)).round() as i32;
            if fw > 4 {
                draw_rounded_box(pixels, width, height, 45, 192, fw, 20, 9.0, COL_LIME, None);
            }
        } else if st.step == 3 {
            let btn_col = if st.hovered_btn == 21 { 0x00_CCFF44 } else { COL_LIME };
            draw_rounded_box(
                pixels,
                width,
                height,
                width - 166,
                344,
                142,
                42,
                10.0,
                btn_col,
                None,
            );
        }
    }

    SetBkMode(mem_dc, TRANSPARENT);
    let f_title = make_font(24, FW_BOLD.0 as i32);
    let f_bold = make_font(15, FW_SEMIBOLD.0 as i32);
    let f_body = make_font(14, FW_NORMAL.0 as i32);
    let old_font = SelectObject(mem_dc, f_title);

    draw_text_rect(
        mem_dc,
        "I Connect Server Setup",
        24,
        18,
        420,
        30,
        COL_TEXT_MAIN,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    SelectObject(mem_dc, f_body);
    draw_text_rect(
        mem_dc,
        "Native Rust Windows Installer  •  AMOLED Lime Edition",
        24,
        48,
        460,
        22,
        COL_LIME,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    if st.step == 0 {
        SelectObject(mem_dc, f_bold);
        draw_text_rect(
            mem_dc,
            "Installation Settings:",
            45,
            98,
            300,
            24,
            COL_TEXT_MAIN,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        SelectObject(mem_dc, f_body);
        let path_line = format!("Install Path: {}", st.install_dir.display());
        draw_text_rect(
            mem_dc,
            &path_line,
            45,
            128,
            width - 90,
            22,
            COL_TEXT_MUTED,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );

        SelectObject(mem_dc, f_bold);
        draw_text_rect(
            mem_dc,
            if st.opt_autostart {
                "[X]  Start automatically with Windows boot (Recommended)"
            } else {
                "[  ]  Start automatically with Windows boot"
            },
            45,
            168,
            460,
            24,
            if st.opt_autostart { COL_LIME } else { COL_TEXT_MUTED },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        draw_text_rect(
            mem_dc,
            if st.opt_desktop_shortcut {
                "[X]  Create Desktop Shortcut icon"
            } else {
                "[  ]  Create Desktop Shortcut icon"
            },
            45,
            204,
            420,
            24,
            if st.opt_desktop_shortcut { COL_LIME_DIM } else { COL_TEXT_MUTED },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        draw_text_rect(
            mem_dc,
            if st.opt_start_menu {
                "[X]  Create Start Menu Program Entry"
            } else {
                "[  ]  Create Start Menu Program Entry"
            },
            45,
            240,
            420,
            24,
            if st.opt_start_menu { COL_LIME_DIM } else { COL_TEXT_MUTED },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        SelectObject(mem_dc, f_body);
        draw_text_rect(
            mem_dc,
            "Click 'Install Now' to proceed with native installation.",
            45,
            284,
            450,
            22,
            COL_TEXT_MUTED,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        SelectObject(mem_dc, f_bold);
        draw_text_rect(
            mem_dc,
            "Install Now >>",
            width - 190,
            344,
            166,
            42,
            0x00_000000,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    } else if st.step == 2 {
        SelectObject(mem_dc, f_bold);
        draw_text_rect(
            mem_dc,
            "Installing I Connect Server...",
            45,
            118,
            400,
            24,
            COL_TEXT_MAIN,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(mem_dc, f_body);
        draw_text_rect(
            mem_dc,
            &st.status_text,
            45,
            150,
            width - 90,
            22,
            COL_LIME,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
    } else if st.step == 3 {
        SelectObject(mem_dc, f_bold);
        draw_text_rect(
            mem_dc,
            "Installation Complete!",
            45,
            108,
            400,
            24,
            COL_LIME,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(mem_dc, f_body);
        draw_text_rect(
            mem_dc,
            "I Connect Server is now installed on your system.",
            45,
            144,
            480,
            22,
            COL_TEXT_MAIN,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        draw_text_rect(
            mem_dc,
            "Auto-start on Windows boot and Firewall rules are configured.",
            45,
            174,
            480,
            22,
            COL_TEXT_MAIN,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        draw_text_rect(
            mem_dc,
            "Closing the server window minimizes it silently to the System Tray.",
            45,
            204,
            480,
            22,
            COL_TEXT_MAIN,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        SelectObject(mem_dc, f_bold);
        draw_text_rect(
            mem_dc,
            if st.opt_launch_now {
                "[X]  Launch I Connect Server now"
            } else {
                "[  ]  Launch I Connect Server now"
            },
            45,
            248,
            420,
            24,
            if st.opt_launch_now { COL_LIME } else { COL_TEXT_MUTED },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        draw_text_rect(
            mem_dc,
            "Finish",
            width - 166,
            344,
            142,
            42,
            0x00_000000,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }

    let _ = BitBlt(hdc, 0, 0, width, height, mem_dc, 0, 0, SRCCOPY);

    SelectObject(mem_dc, old_font);
    let _ = DeleteObject(f_title);
    let _ = DeleteObject(f_bold);
    let _ = DeleteObject(f_body);
    SelectObject(mem_dc, old_bmp);
    let _ = DeleteObject(HGDIOBJ(dib.0));
    let _ = DeleteDC(mem_dc);
}

unsafe extern "system" fn setup_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            if let Ok(mut s) = state().lock() {
                s.hwnd_raw = hwnd.0 as isize;
            }
            LRESULT(0)
        }
        WM_APP_REPAINT => {
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let mut s = state().lock().unwrap();
            let hit = hit_test(rc.right - rc.left, x, y, s.step);
            if let Ok(cur) = LoadCursorW(None, if hit >= 0 { IDC_HAND } else { IDC_ARROW }) {
                SetCursor(cur);
            }
            if s.hovered_btn != hit {
                s.hovered_btn = hit;
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);

            let action = {
                let mut s = state().lock().unwrap();
                let hit = hit_test(rc.right - rc.left, x, y, s.step);
                match hit {
                    10 => {
                        s.opt_autostart = !s.opt_autostart;
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                    11 => {
                        s.opt_desktop_shortcut = !s.opt_desktop_shortcut;
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                    12 => {
                        s.opt_start_menu = !s.opt_start_menu;
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                    20 => {
                        s.opt_launch_now = !s.opt_launch_now;
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                    _ => {}
                }
                hit
            };

            if action == 13 {
                start_installation_worker();
            } else if action == 21 {
                let (launch, dir) = {
                    let s = state().lock().unwrap();
                    (s.opt_launch_now, s.install_dir.clone())
                };
                if launch {
                    let exe_dest = dir.join("I_Connect_Server.exe");
                    let open = HSTRING::from("open");
                    let p = HSTRING::from(exe_dest.to_string_lossy().as_ref());
                    ShellExecuteW(
                        HWND::default(),
                        PCWSTR(open.as_ptr()),
                        PCWSTR(p.as_ptr()),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOWNORMAL,
                    );
                }
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Ok(s) = state().lock() {
                paint_installer(hwnd, hdc, &s);
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    let install_dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("Programs").join("IConnect"))
        .unwrap_or_else(|| PathBuf::from(r"C:\IConnect"));

    let _ = STATE.set(Mutex::new(InstallerState {
        hwnd_raw: 0,
        step: 0,
        install_dir,
        opt_autostart: true,
        opt_desktop_shortcut: true,
        opt_start_menu: true,
        opt_launch_now: true,
        progress_pct: 0,
        status_text: String::new(),
        hovered_btn: -1,
    }));

    unsafe {
        let h_inst_mod = GetModuleHandleW(None).unwrap_or_default();
        let h_inst = HINSTANCE(h_inst_mod.0);
        let h_icon = LoadIconW(h_inst, PCWSTR(101usize as *const u16))
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default();
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();

        let cls = HSTRING::from("IConnectInstallerClass");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(setup_wnd_proc),
            hInstance: h_inst,
            hIcon: h_icon,
            hCursor: cursor,
            lpszClassName: PCWSTR(cls.as_ptr()),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return;
        }

        let win_w = 590;
        let win_h = 440;
        let scr_w = GetSystemMetrics(SM_CXSCREEN);
        let scr_h = GetSystemMetrics(SM_CYSCREEN);
        let title = HSTRING::from("I Connect Server Setup Wizard");

        let Ok(hwnd) = CreateWindowExW(
            Default::default(),
            PCWSTR(cls.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            (scr_w - win_w) / 2,
            (scr_h - win_h) / 2,
            win_w,
            win_h,
            None,
            None,
            h_inst,
            None,
        ) else {
            return;
        };

        let dark_mode = BOOL(1);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_mode as *const BOOL as *const _,
            size_of::<BOOL>() as u32,
        );

        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
