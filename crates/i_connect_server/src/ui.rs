use crate::state::{is_autostart_enabled, AppState};
use i_connect_proto::{REMOTE_CONTROL_PORT, REMOTE_FILE_PORT};
use std::mem::size_of;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
    SelectObject, SetBkMode, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DRAW_TEXT_FORMAT,
    DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_BOLD,
    FW_NORMAL, FW_SEMIBOLD, HDC, HGDIOBJ, OUT_DEFAULT_PRECIS, SRCCOPY, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

// Pure AMOLED Black + Electric Lime Theme Palette
const COL_BG_AMOLED: u32 = 0x00_000000;
const COL_CARD_BG: u32 = 0x00_0A0D08;
const COL_CARD_BORDER: u32 = 0x00_1F2E14;
const COL_DROP_BG: u32 = 0x00_050804;
const COL_LIME: u32 = 0x00_B6FF00;
const COL_LIME_MID: u32 = 0x00_84CC16;
const COL_LIME_SOFT_BG: u32 = 0x00_101A07;
const COL_BTN_BG: u32 = 0x00_10150D;
const COL_BTN_HOVER: u32 = 0x00_1A2413;
const COL_PROGRESS_BG: u32 = 0x00_141C0F;
const COL_TEXT_MAIN: u32 = 0x00_F4FCE8;
const COL_TEXT_MUTED: u32 = 0x00_889A78;
const COL_AMBER: u32 = 0x00_FFB020;
const COL_RED: u32 = 0x00_FF4D5E;

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
    let br = ((bg >> 16) & 0xFF) as f32;
    let bg_c = ((bg >> 8) & 0xFF) as f32;
    let bb = (bg & 0xFF) as f32;

    let fr = ((fg >> 16) & 0xFF) as f32;
    let fg_c = ((fg >> 8) & 0xFF) as f32;
    let fb = (fg & 0xFF) as f32;

    let r = (br * inv + fr * a).round() as u32;
    let g = (bg_c * inv + fg_c * a).round() as u32;
    let b = (bb * inv + fb * a).round() as u32;
    (r << 16) | (g << 8) | b
}

/// Signed distance to a rounded box centered at (cx, cy) with half-extents (hx, hy) and corner radius r.
#[inline]
fn sd_rounded_box(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let dx = (px - cx).abs() - hx + r;
    let dy = (py - cy).abs() - hy + r;
    let ox = dx.max(0.0);
    let oy = dy.max(0.0);
    (ox * ox + oy * oy).sqrt() + dx.max(dy).min(0.0) - r
}

struct PixelCanvas<'a> {
    pixels: &'a mut [u32],
    w: i32,
    h: i32,
}

impl<'a> PixelCanvas<'a> {
    fn clear(&mut self, col: u32) {
        self.pixels.fill(col);
    }

    fn blend_at(&mut self, x: i32, y: i32, col: u32, alpha: f32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || alpha <= 0.002 {
            return;
        }
        let idx = (y * self.w + x) as usize;
        self.pixels[idx] = blend_px(self.pixels[idx], col, alpha);
    }

    fn fill_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: f32, fill: u32, border: Option<(u32, f32)>) {
        let x0 = (x - 2).max(0);
        let y0 = (y - 2).max(0);
        let x1 = (x + w + 2).min(self.w - 1);
        let y1 = (y + h + 2).min(self.h - 1);
        let cx = x as f32 + w as f32 * 0.5;
        let cy = y as f32 + h as f32 * 0.5;
        let hx = w as f32 * 0.5;
        let hy = h as f32 * 0.5;
        let rr = r.min(hx).min(hy);

        for py in y0..=y1 {
            for px in x0..=x1 {
                let d = sd_rounded_box(px as f32 + 0.5, py as f32 + 0.5, cx, cy, hx, hy, rr);
                if d < 1.0 {
                    let fill_alpha = (0.5 - d).clamp(0.0, 1.0);
                    self.blend_at(px, py, fill, fill_alpha);
                    if let Some((bcol, bwidth)) = border {
                        let bd = (d + bwidth * 0.5).abs() - bwidth * 0.5;
                        let ba = (0.5 - bd).clamp(0.0, 1.0);
                        self.blend_at(px, py, bcol, ba);
                    }
                } else if let Some((bcol, bwidth)) = border {
                    let bd = (d + bwidth * 0.5).abs() - bwidth * 0.5;
                    if bd < 0.75 {
                        let ba = (0.5 - bd).clamp(0.0, 1.0);
                        self.blend_at(px, py, bcol, ba);
                    }
                }
            }
        }
    }

    fn draw_dashed_rounded_rect(&mut self, x: i32, y: i32, w: i32, h: i32, r: f32, fill: u32, border: u32) {
        self.fill_rounded_rect(x, y, w, h, r, fill, None);
        let x0 = (x - 2).max(0);
        let y0 = (y - 2).max(0);
        let x1 = (x + w + 2).min(self.w - 1);
        let y1 = (y + h + 2).min(self.h - 1);
        let cx = x as f32 + w as f32 * 0.5;
        let cy = y as f32 + h as f32 * 0.5;
        let hx = w as f32 * 0.5;
        let hy = h as f32 * 0.5;
        let rr = r.min(hx).min(hy);

        for py in y0..=y1 {
            for px in x0..=x1 {
                let fx = px as f32 + 0.5;
                let fy = py as f32 + 0.5;
                let d = sd_rounded_box(fx, fy, cx, cy, hx, hy, rr);
                let bd = d.abs() - 0.85;
                if bd < 0.6 {
                    // Perimeter dash modulation using Manhattan phase around the perimeter
                    let phase = ((px - x) + (py - y)) % 18;
                    let on_dash = phase < 10;
                    if on_dash {
                        let ba = (0.5 - bd).clamp(0.0, 1.0);
                        self.blend_at(px, py, border, ba);
                    }
                }
            }
        }
    }

    fn draw_glow_dot(&mut self, cx: f32, cy: f32, radius: f32, glow_radius: f32, col: u32) {
        let x0 = (cx - glow_radius - 1.0).floor().max(0.0) as i32;
        let y0 = (cy - glow_radius - 1.0).floor().max(0.0) as i32;
        let x1 = (cx + glow_radius + 1.0).ceil().min((self.w - 1) as f32) as i32;
        let y1 = (cy + glow_radius + 1.0).ceil().min((self.h - 1) as f32) as i32;

        for py in y0..=y1 {
            for px in x0..=x1 {
                let dx = px as f32 + 0.5 - cx;
                let dy = py as f32 + 0.5 - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= radius + 0.8 {
                    let a = (radius + 0.5 - dist).clamp(0.0, 1.0);
                    self.blend_at(px, py, col, a);
                } else if dist < glow_radius {
                    let t = 1.0 - (dist - radius) / (glow_radius - radius);
                    self.blend_at(px, py, col, t * t * 0.45);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct HitButtons {
    pub open_downloads: (i32, i32, i32, i32),
    pub win_boot: (i32, i32, i32, i32),
    pub reset_paired: (i32, i32, i32, i32),
    pub ip_card: (i32, i32, i32, i32),
}

pub fn get_button_rects(width: i32) -> HitButtons {
    let cards_y = 68;
    let card_h = 104;
    let drop_y = cards_y + card_h + 16;
    let drop_h = 194;
    let btn_y = drop_y + drop_h + 18;
    let card_w = (width - 48 - 16) / 2;
    HitButtons {
        open_downloads: (24, btn_y, 204, btn_y + 40),
        win_boot: (216, btn_y, 438, btn_y + 40),
        reset_paired: (450, btn_y, width - 24, btn_y + 40),
        ip_card: (24, cards_y, 24 + card_w, cards_y + card_h),
    }
}

pub fn hit_test_button(width: i32, x: i32, y: i32) -> i32 {
    let b = get_button_rects(width);
    let inside = |r: (i32, i32, i32, i32)| x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3;
    if inside(b.open_downloads) {
        0
    } else if inside(b.win_boot) {
        1
    } else if inside(b.reset_paired) {
        2
    } else if inside(b.ip_card) {
        3
    } else {
        -1
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

pub unsafe fn paint_main_window(hwnd: HWND, hdc: HDC, state: &AppState, copied_flash: bool) {
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let width = (rc.right - rc.left).max(1);
    let height = (rc.bottom - rc.top).max(1);

    let mem_dc = CreateCompatibleDC(hdc);
    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height, // Top-down DIB
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };

    let mut bits_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
    let Ok(dib) = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits_ptr, None, 0) else {
        let _ = DeleteDC(mem_dc);
        return;
    };
    if bits_ptr.is_null() {
        let _ = DeleteObject(HGDIOBJ(dib.0));
        let _ = DeleteDC(mem_dc);
        return;
    }

    let old_bmp = SelectObject(mem_dc, HGDIOBJ(dib.0));

    // 1. Vector-crisp anti-aliased AMOLED + Lime geometry pass
    {
        let len = (width * height) as usize;
        let pixels = std::slice::from_raw_parts_mut(bits_ptr as *mut u32, len);
        let mut canvas = PixelCanvas {
            pixels,
            w: width,
            h: height,
        };
        canvas.clear(COL_BG_AMOLED);

        // Status pill top-right
        let badge_w = 192;
        let badge_x = width - 24 - badge_w;
        let badge_y = 18;
        let status_col = if state.bind_error.is_some() {
            COL_RED
        } else if state.is_connected {
            COL_LIME
        } else {
            COL_AMBER
        };
        canvas.fill_rounded_rect(
            badge_x,
            badge_y,
            badge_w,
            32,
            10.0,
            if state.bind_error.is_some() {
                0x00_20080B
            } else if state.is_connected {
                COL_LIME_SOFT_BG
            } else {
                0x00_181104
            },
            Some((status_col, 1.3)),
        );
        canvas.draw_glow_dot(
            (badge_x + 18) as f32,
            (badge_y + 16) as f32,
            4.5,
            11.0,
            status_col,
        );

        // Top cards
        let card_w = (width - 48 - 16) / 2;
        let card_h = 104;
        let card1_x = 24;
        let card2_x = 24 + card_w + 16;
        let cards_y = 68;

        let ip_border = if state.hovered_btn == 3 {
            COL_LIME
        } else {
            COL_CARD_BORDER
        };
        canvas.fill_rounded_rect(
            card1_x,
            cards_y,
            card_w,
            card_h,
            14.0,
            COL_CARD_BG,
            Some((ip_border, 1.2)),
        );

        let dev_border = if state.is_connected {
            COL_LIME_MID
        } else {
            COL_CARD_BORDER
        };
        canvas.fill_rounded_rect(
            card2_x,
            cards_y,
            card_w,
            card_h,
            14.0,
            COL_CARD_BG,
            Some((dev_border, 1.2)),
        );

        // Drag & drop zone
        let drop_y = cards_y + card_h + 16;
        let drop_w = width - 48;
        let drop_h = 194;
        let drop_border = if state.is_connected {
            COL_LIME
        } else {
            0x00_2B3D1D
        };
        canvas.draw_dashed_rounded_rect(24, drop_y, drop_w, drop_h, 16.0, COL_DROP_BG, drop_border);

        // Transfer progress bar
        if state.transfer.is_transferring {
            let pb_x = 48;
            let pb_y = drop_y + 114;
            let pb_w = drop_w - 48;
            let pb_h = 16;
            canvas.fill_rounded_rect(pb_x, pb_y, pb_w, pb_h, 8.0, COL_PROGRESS_BG, Some((0x00_27381B, 1.0)));

            let pct = if state.transfer.current_file_size > 0 {
                (state.transfer.current_file_transferred as f32
                    / state.transfer.current_file_size as f32)
                    .clamp(0.0, 1.0)
            } else {
                0.0
            };
            let fill_w = (pb_w as f32 * pct).round() as i32;
            if fill_w > 4 {
                canvas.fill_rounded_rect(pb_x, pb_y, fill_w, pb_h, 8.0, COL_LIME, None);
            }
        }

        // Bottom 3 action buttons
        let rects = get_button_rects(width);
        let auto_on = is_autostart_enabled();

        let btn0_bg = if state.hovered_btn == 0 { COL_BTN_HOVER } else { COL_BTN_BG };
        let btn0_bd = if state.hovered_btn == 0 { COL_LIME } else { COL_CARD_BORDER };
        canvas.fill_rounded_rect(
            rects.open_downloads.0,
            rects.open_downloads.1,
            rects.open_downloads.2 - rects.open_downloads.0,
            40,
            10.0,
            btn0_bg,
            Some((btn0_bd, 1.2)),
        );

        let btn1_bg = if state.hovered_btn == 1 { COL_BTN_HOVER } else { COL_BTN_BG };
        let btn1_bd = if auto_on {
            COL_LIME
        } else if state.hovered_btn == 1 {
            COL_LIME_MID
        } else {
            COL_CARD_BORDER
        };
        canvas.fill_rounded_rect(
            rects.win_boot.0,
            rects.win_boot.1,
            rects.win_boot.2 - rects.win_boot.0,
            40,
            10.0,
            btn1_bg,
            Some((btn1_bd, 1.2)),
        );

        let btn2_bg = if state.hovered_btn == 2 { 0x00_1D0A0C } else { COL_BTN_BG };
        let btn2_bd = if state.hovered_btn == 2 { COL_RED } else { COL_CARD_BORDER };
        canvas.fill_rounded_rect(
            rects.reset_paired.0,
            rects.reset_paired.1,
            rects.reset_paired.2 - rects.reset_paired.0,
            40,
            10.0,
            btn2_bg,
            Some((btn2_bd, 1.2)),
        );
    }

    // 2. Crisp ClearType Typography Pass
    SetBkMode(mem_dc, TRANSPARENT);
    let f_title = make_font(26, FW_BOLD.0 as i32);
    let f_big = make_font(21, FW_BOLD.0 as i32);
    let f_bold = make_font(15, FW_SEMIBOLD.0 as i32);
    let f_sub = make_font(13, FW_NORMAL.0 as i32);
    let old_font = SelectObject(mem_dc, f_title);

    // Header Title + Badge
    draw_text_rect(mem_dc, "I CONNECT", 24, 16, 150, 34, COL_LIME, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
    SelectObject(mem_dc, f_sub);
    draw_text_rect(
        mem_dc,
        "[TURBO LINK  •  WI-FI / HOTSPOT]",
        166,
        20,
        240,
        28,
        COL_TEXT_MUTED,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let badge_w = 192;
    let badge_x = width - 24 - badge_w;
    let status_col = if state.bind_error.is_some() {
        COL_RED
    } else if state.is_connected {
        COL_LIME
    } else {
        COL_AMBER
    };
    SelectObject(mem_dc, f_bold);
    draw_text_rect(
        mem_dc,
        if state.bind_error.is_some() {
            "PORT CONFLICT!"
        } else if state.is_connected {
            "PHONE CONNECTED"
        } else {
            "LISTENING (READY)"
        },
        badge_x + 28,
        18,
        badge_w - 34,
        32,
        status_col,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    // Cards Text
    let card_w = (width - 48 - 16) / 2;
    let card1_x = 24;
    let card2_x = 24 + card_w + 16;
    let cards_y = 68;

    SelectObject(mem_dc, f_sub);
    draw_text_rect(
        mem_dc,
        if copied_flash { "IP COPIED TO CLIPBOARD!" } else { "LOCAL PC IP (CLICK TO COPY)" },
        card1_x + 16,
        cards_y + 12,
        card_w - 32,
        20,
        if copied_flash { COL_LIME } else { COL_TEXT_MUTED },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    SelectObject(mem_dc, f_big);
    draw_text_rect(
        mem_dc,
        &state.local_ip,
        card1_x + 16,
        cards_y + 36,
        card_w - 32,
        30,
        COL_LIME,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );
    SelectObject(mem_dc, f_sub);
    let port_line = format!("Ports: {} (Control)  |  {} (Files)", REMOTE_CONTROL_PORT, REMOTE_FILE_PORT);
    draw_text_rect(
        mem_dc,
        &port_line,
        card1_x + 16,
        cards_y + 72,
        card_w - 32,
        20,
        COL_TEXT_MUTED,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    draw_text_rect(
        mem_dc,
        "CONNECTED DEVICE",
        card2_x + 16,
        cards_y + 12,
        card_w - 32,
        20,
        COL_TEXT_MUTED,
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    SelectObject(mem_dc, f_big);
    draw_text_rect(
        mem_dc,
        if state.is_connected { &state.client_name } else { "Waiting for phone..." },
        card2_x + 16,
        cards_y + 36,
        card_w - 32,
        30,
        if state.is_connected { COL_TEXT_MAIN } else { COL_TEXT_MUTED },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );
    SelectObject(mem_dc, f_sub);
    let dev_info = if let Some(ref err) = state.bind_error {
        err.clone()
    } else if state.is_connected {
        format!("IP: {}  |  Pkts: {}", state.client_ip, state.total_packets_received)
    } else {
        format!("Paired whitelist: {} device(s) • Silent reconnect", state.paired_count)
    };
    draw_text_rect(
        mem_dc,
        &dev_info,
        card2_x + 16,
        cards_y + 72,
        card_w - 32,
        20,
        if state.bind_error.is_some() { COL_RED } else { COL_TEXT_MUTED },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
    );

    // Drop zone text
    let drop_y = cards_y + 104 + 16;
    let drop_w = width - 48;
    SelectObject(mem_dc, f_big);
    let drop_title = if state.is_connected {
        ">> DRAG & DROP FILES OR FOLDERS TO SEND TO PHONE <<"
    } else {
        "CONNECT PHONE TO SEND FILES & FOLDERS VIA DRAG & DROP"
    };
    draw_text_rect(
        mem_dc,
        drop_title,
        32,
        drop_y + 38,
        drop_w - 16,
        30,
        if state.is_connected { COL_LIME } else { COL_TEXT_MUTED },
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    SelectObject(mem_dc, f_sub);
    draw_text_rect(
        mem_dc,
        "Supports batch files & nested folder trees at full local Wi-Fi / Hotspot speed",
        32,
        drop_y + 74,
        drop_w - 16,
        22,
        COL_TEXT_MUTED,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    if state.transfer.is_transferring {
        let pct = if state.transfer.current_file_size > 0 {
            (state.transfer.current_file_transferred as f32
                / state.transfer.current_file_size as f32)
                .clamp(0.0, 1.0)
                * 100.0
        } else {
            0.0
        };
        let direction = if state.transfer.is_sending { "Sending" } else { "Receiving" };
        let stats_line = format!(
            "{} [{}/{}]: {} ({:.1}%)  |  {:.1} MB/s",
            direction,
            state.transfer.current_file_index,
            state.transfer.total_files_in_batch,
            state.transfer.current_file_name,
            pct,
            state.transfer.current_speed_mbps
        );
        draw_text_rect(
            mem_dc,
            &stats_line,
            48,
            drop_y + 138,
            drop_w - 48,
            24,
            COL_LIME,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    } else {
        draw_text_rect(
            mem_dc,
            "Incoming phone files save automatically to Downloads\\I_Connect_Transfers",
            32,
            drop_y + 122,
            drop_w - 16,
            22,
            COL_TEXT_MUTED,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
    }

    // Bottom buttons text
    let rects = get_button_rects(width);
    let auto_on = is_autostart_enabled();
    SelectObject(mem_dc, f_bold);

    draw_text_rect(
        mem_dc,
        "Open Downloads",
        rects.open_downloads.0,
        rects.open_downloads.1,
        rects.open_downloads.2 - rects.open_downloads.0,
        40,
        COL_TEXT_MAIN,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    let boot_label = if auto_on { "Windows Boot: [ON]" } else { "Windows Boot: [OFF]" };
    draw_text_rect(
        mem_dc,
        boot_label,
        rects.win_boot.0,
        rects.win_boot.1,
        rects.win_boot.2 - rects.win_boot.0,
        40,
        if auto_on { COL_LIME } else { COL_TEXT_MUTED },
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    draw_text_rect(
        mem_dc,
        "Reset Paired Phones",
        rects.reset_paired.0,
        rects.reset_paired.1,
        rects.reset_paired.2 - rects.reset_paired.0,
        40,
        COL_RED,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    let _ = BitBlt(hdc, 0, 0, width, height, mem_dc, 0, 0, SRCCOPY);

    SelectObject(mem_dc, old_font);
    let _ = DeleteObject(f_title);
    let _ = DeleteObject(f_big);
    let _ = DeleteObject(f_bold);
    let _ = DeleteObject(f_sub);

    SelectObject(mem_dc, old_bmp);
    let _ = DeleteObject(HGDIOBJ(dib.0));
    let _ = DeleteDC(mem_dc);
}
