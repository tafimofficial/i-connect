use i_connect_proto::{MediaAction, MouseAction, MouseButton, SysCommand};
use std::mem::size_of;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::thread;
use std::time::Duration;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{BOOLEAN, HWND};
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, VkKeyScanW, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE,
    KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    MAPVK_VK_TO_VSC, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT, VIRTUAL_KEY, VK_CONTROL, VK_DELETE,
    VK_DIVIDE, VK_DOWN, VK_END, VK_F4, VK_HOME, VK_INSERT, VK_LEFT, VK_LWIN, VK_MEDIA_NEXT_TRACK,
    VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MENU, VK_NEXT, VK_NUMLOCK, VK_PRIOR, VK_RCONTROL,
    VK_RIGHT, VK_RMENU, VK_RWIN, VK_SHIFT, VK_UP, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const CREATE_NO_WINDOW: u32 = 0x08000000;

fn mouse_input(dx: i32, dy: i32, mouse_data: u32, flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn key_input(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn scan_for(vk: VIRTUAL_KEY) -> u16 {
    unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) as u16 }
}

fn extended_flag(vk: VIRTUAL_KEY) -> KEYBD_EVENT_FLAGS {
    match vk {
        VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_INSERT | VK_DELETE | VK_HOME | VK_END
        | VK_PRIOR | VK_NEXT | VK_RCONTROL | VK_RMENU | VK_LWIN | VK_RWIN | VK_DIVIDE
        | VK_NUMLOCK | VK_MEDIA_PLAY_PAUSE | VK_MEDIA_PREV_TRACK | VK_MEDIA_NEXT_TRACK
        | VK_VOLUME_UP | VK_VOLUME_DOWN | VK_VOLUME_MUTE => KEYEVENTF_EXTENDEDKEY,
        _ => KEYBD_EVENT_FLAGS(0),
    }
}

pub fn simulate_mouse_move(dx: i16, dy: i16) {
    let inp = [mouse_input(dx as i32, dy as i32, 0, MOUSEEVENTF_MOVE)];
    unsafe {
        SendInput(&inp, size_of::<INPUT>() as i32);
    }
}

pub fn simulate_mouse_click(button: u8, action: u8) {
    let (down_flag, up_flag) = match MouseButton::from_u8(button) {
        Some(MouseButton::Left) => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        Some(MouseButton::Right) => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        Some(MouseButton::Middle) => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        None => return,
    };

    unsafe {
        match MouseAction::from_u8(action) {
            Some(MouseAction::Down) => {
                SendInput(&[mouse_input(0, 0, 0, down_flag)], size_of::<INPUT>() as i32);
            }
            Some(MouseAction::Up) => {
                SendInput(&[mouse_input(0, 0, 0, up_flag)], size_of::<INPUT>() as i32);
            }
            Some(MouseAction::Click) => {
                SendInput(
                    &[mouse_input(0, 0, 0, down_flag), mouse_input(0, 0, 0, up_flag)],
                    size_of::<INPUT>() as i32,
                );
            }
            Some(MouseAction::DoubleClick) => {
                SendInput(
                    &[mouse_input(0, 0, 0, down_flag), mouse_input(0, 0, 0, up_flag)],
                    size_of::<INPUT>() as i32,
                );
                thread::sleep(Duration::from_millis(20));
                SendInput(
                    &[mouse_input(0, 0, 0, down_flag), mouse_input(0, 0, 0, up_flag)],
                    size_of::<INPUT>() as i32,
                );
            }
            None => {}
        }
    }
}

pub fn simulate_mouse_scroll(delta_y: i16, delta_x: i16) {
    unsafe {
        if delta_y != 0 {
            let inp = [mouse_input(0, 0, delta_y as i32 as u32, MOUSEEVENTF_WHEEL)];
            SendInput(&inp, size_of::<INPUT>() as i32);
        }
        if delta_x != 0 {
            let inp = [mouse_input(0, 0, delta_x as i32 as u32, MOUSEEVENTF_HWHEEL)];
            SendInput(&inp, size_of::<INPUT>() as i32);
        }
    }
}

pub fn simulate_key_event(vk_code: u16, action: u8, modifiers: u8) {
    let mut inputs = Vec::with_capacity(12);
    let vk = VIRTUAL_KEY(vk_code);
    let scan = scan_for(vk);
    let ext = extended_flag(vk);

    if (modifiers & 1) != 0 && action != 1 {
        inputs.push(key_input(VK_SHIFT, scan_for(VK_SHIFT), KEYBD_EVENT_FLAGS(0)));
    }
    if (modifiers & 2) != 0 && action != 1 {
        inputs.push(key_input(VK_CONTROL, scan_for(VK_CONTROL), KEYBD_EVENT_FLAGS(0)));
    }
    if (modifiers & 4) != 0 && action != 1 {
        inputs.push(key_input(VK_MENU, scan_for(VK_MENU), KEYBD_EVENT_FLAGS(0)));
    }
    if (modifiers & 8) != 0 && action != 1 {
        inputs.push(key_input(VK_LWIN, scan_for(VK_LWIN), KEYEVENTF_EXTENDEDKEY));
    }

    match action {
        0 => inputs.push(key_input(vk, scan, ext)),
        1 => inputs.push(key_input(vk, scan, ext | KEYEVENTF_KEYUP)),
        _ => {
            inputs.push(key_input(vk, scan, ext));
            inputs.push(key_input(vk, scan, ext | KEYEVENTF_KEYUP));
        }
    }

    if (modifiers & 8) != 0 && action != 0 {
        inputs.push(key_input(
            VK_LWIN,
            scan_for(VK_LWIN),
            KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP,
        ));
    }
    if (modifiers & 4) != 0 && action != 0 {
        inputs.push(key_input(VK_MENU, scan_for(VK_MENU), KEYEVENTF_KEYUP));
    }
    if (modifiers & 2) != 0 && action != 0 {
        inputs.push(key_input(VK_CONTROL, scan_for(VK_CONTROL), KEYEVENTF_KEYUP));
    }
    if (modifiers & 1) != 0 && action != 0 {
        inputs.push(key_input(VK_SHIFT, scan_for(VK_SHIFT), KEYEVENTF_KEYUP));
    }

    if !inputs.is_empty() {
        unsafe {
            SendInput(&inputs, size_of::<INPUT>() as i32);
        }
    }
}

pub fn simulate_text_input(text: &str) {
    if text.is_empty() {
        return;
    }
    let utf16: Vec<u16> = text.encode_utf16().collect();
    for ch in utf16 {
        let vk_res = unsafe { VkKeyScanW(ch) };
        if vk_res != -1 {
            let vk_byte = (vk_res & 0xFF) as u16;
            let shift_state = ((vk_res >> 8) & 0xFF) as u8;
            let vk = VIRTUAL_KEY(vk_byte);
            let scan = scan_for(vk);
            let ext = extended_flag(vk);

            let mut inputs = Vec::with_capacity(8);
            if (shift_state & 1) != 0 {
                inputs.push(key_input(VK_SHIFT, scan_for(VK_SHIFT), KEYBD_EVENT_FLAGS(0)));
            }
            if (shift_state & 2) != 0 {
                inputs.push(key_input(VK_CONTROL, scan_for(VK_CONTROL), KEYBD_EVENT_FLAGS(0)));
            }
            if (shift_state & 4) != 0 {
                inputs.push(key_input(VK_MENU, scan_for(VK_MENU), KEYBD_EVENT_FLAGS(0)));
            }

            inputs.push(key_input(vk, scan, ext));
            inputs.push(key_input(vk, scan, ext | KEYEVENTF_KEYUP));

            if (shift_state & 4) != 0 {
                inputs.push(key_input(VK_MENU, scan_for(VK_MENU), KEYEVENTF_KEYUP));
            }
            if (shift_state & 2) != 0 {
                inputs.push(key_input(VK_CONTROL, scan_for(VK_CONTROL), KEYEVENTF_KEYUP));
            }
            if (shift_state & 1) != 0 {
                inputs.push(key_input(VK_SHIFT, scan_for(VK_SHIFT), KEYEVENTF_KEYUP));
            }

            unsafe {
                SendInput(&inputs, size_of::<INPUT>() as i32);
            }
        } else {
            let inputs = [
                key_input(VIRTUAL_KEY(0), ch, KEYEVENTF_UNICODE),
                key_input(VIRTUAL_KEY(0), ch, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
            ];
            unsafe {
                SendInput(&inputs, size_of::<INPUT>() as i32);
            }
        }
    }
}

fn press_vk(vk: VIRTUAL_KEY) {
    let scan = scan_for(vk);
    let ext = extended_flag(vk);
    let inputs = [
        key_input(vk, scan, ext),
        key_input(vk, scan, ext | KEYEVENTF_KEYUP),
    ];
    unsafe {
        SendInput(&inputs, size_of::<INPUT>() as i32);
    }
}

pub fn execute_media_command(action: u8, value: u8) {
    match MediaAction::from_u8(action) {
        Some(MediaAction::PlayPause) => press_vk(VK_MEDIA_PLAY_PAUSE),
        Some(MediaAction::Prev) => press_vk(VK_MEDIA_PREV_TRACK),
        Some(MediaAction::Next) => press_vk(VK_MEDIA_NEXT_TRACK),
        Some(MediaAction::VolUp) => press_vk(VK_VOLUME_UP),
        Some(MediaAction::VolDown) => press_vk(VK_VOLUME_DOWN),
        Some(MediaAction::Mute) => press_vk(VK_VOLUME_MUTE),
        Some(MediaAction::SeekBack) => press_vk(VK_LEFT),
        Some(MediaAction::SeekFwd) => press_vk(VK_RIGHT),
        Some(MediaAction::SeekPos) => {
            if value <= 100 {
                let digit = (value / 10).min(9);
                press_vk(VIRTUAL_KEY(b'0' as u16 + digit as u16));
            }
        }
        None => {}
    }
}

pub fn execute_system_command(cmd: u8) {
    match SysCommand::from_u8(cmd) {
        Some(SysCommand::Lock) => unsafe {
            let _ = LockWorkStation();
        },
        Some(SysCommand::Sleep) => unsafe {
            SetSuspendState(BOOLEAN(0), BOOLEAN(0), BOOLEAN(0));
        },
        Some(SysCommand::Shutdown) => {
            let _ = Command::new("shutdown")
                .args(["/s", "/t", "10", "/c", "Remote Shutdown initiated from Phone"])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
        }
        Some(SysCommand::Restart) => {
            let _ = Command::new("shutdown")
                .args(["/r", "/t", "10", "/c", "Remote Restart initiated from Phone"])
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
        }
        Some(SysCommand::ShowDesktop) => {
            // Win + D
            simulate_key_event(b'D' as u16, 2, 8);
        }
        Some(SysCommand::CloseWindow) => {
            // Alt + F4
            simulate_key_event(VK_F4.0, 2, 4);
        }
        Some(SysCommand::TaskManager) => unsafe {
            let open = HSTRING::from("open");
            let file = HSTRING::from("taskmgr.exe");
            ShellExecuteW(
                HWND::default(),
                PCWSTR(open.as_ptr()),
                PCWSTR(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        },
        None => {}
    }
}
