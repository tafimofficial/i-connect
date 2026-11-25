use crate::input::{
    execute_media_command, execute_system_command, simulate_key_event, simulate_mouse_click,
    simulate_mouse_move, simulate_mouse_scroll, simulate_text_input,
};
use crate::state::{
    hwnd_from_raw, is_device_paired, save_device_to_whitelist, RunningFlag, SharedState,
};
use i_connect_proto::{
    sanitize_relative_path, send_frame, CHUNK_BUFFER_SIZE, DISCOVER_PING_MAGIC,
    DISCOVER_PONG_MAGIC, PROTO_MAGIC, REMOTE_CONTROL_PORT, REMOTE_FILE_PORT, REMOTE_UDP_PORT,
    SOCKET_BUFFER_SIZE, STRIPE_SIZE,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4, TcpListener, TcpStream, UdpSocket};
use std::os::windows::fs::FileExt;
use std::os::windows::io::AsRawSocket;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, TcpKeepalive, Type};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::NetworkManagement::IpHelper::{GetAdaptersInfo, IP_ADAPTER_INFO};
use windows::Win32::Networking::WinSock::{setsockopt, SOCKET, SOL_SOCKET, SO_EXCLUSIVEADDRUSE};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, PostMessageW, IDYES, MB_ICONEXCLAMATION, MB_ICONQUESTION, MB_OK, MB_SETFOREGROUND,
    MB_TOPMOST, MB_YESNO, WM_USER,
};

pub const WM_APP_RESTORE: u32 = WM_USER + 201;
pub const WM_APP_TRAY_CONNECTED: u32 = WM_USER + 202;
pub const WM_APP_TRAY_DISCONNECTED: u32 = WM_USER + 203;
pub const WM_APP_TRAY_FILE_DONE: u32 = WM_USER + 204;

// Note: protocol constants imported cleanly
#[allow(unused_imports)]
use i_connect_proto::{
    MsgAuthReq, MsgAuthResp, MsgFileBatchHeader, MsgFileChannelHello, MsgFileItemHeader,
    MsgFileRangeHeader, MsgKeyEvent, MsgMediaCmd, MsgMouseClick, MsgMouseMove, MsgMouseScroll,
    MsgSysCmd, MsgTextInput, MsgType, RemoteHeader,
};

fn cstr_from_i8(buf: &[i8]) -> String {
    unsafe {
        let bytes = std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len());
        let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..len]).into_owned()
    }
}

fn is_virtual_adapter(desc: &str) -> bool {
    let d = desc.to_ascii_lowercase();
    d.contains("vmware")
        || d.contains("virtual")
        || d.contains("loopback")
        || d.contains("bluetooth")
        || d.contains("npcap")
        || d.contains("tap")
        || d.contains("vpn")
        || d.contains("host-only")
}

/// Detect the active physical Wi-Fi / Hotspot IPv4 address and default gateway,
/// automatically filtering out VMware, VirtualBox, VPN, Bluetooth, and Loopback adapters.
pub fn get_local_network_info() -> (String, String) {
    let mut out_ip = "127.0.0.1".to_string();
    let mut out_gw = String::new();

    unsafe {
        let mut buf_len: u32 = std::mem::size_of::<IP_ADAPTER_INFO>() as u32;
        let mut buffer = vec![0u8; buf_len as usize];
        let mut res = GetAdaptersInfo(
            Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_INFO),
            &mut buf_len,
        );
        if res == ERROR_BUFFER_OVERFLOW.0 {
            buffer.resize(buf_len as usize, 0);
            res = GetAdaptersInfo(
                Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_INFO),
                &mut buf_len,
            );
        }
        if res != ERROR_SUCCESS.0 || buffer.is_empty() {
            return (out_ip, out_gw);
        }

        // Pass 1: non-virtual adapter with both valid gateway and valid IP
        let mut ptr = buffer.as_ptr() as *const IP_ADAPTER_INFO;
        while !ptr.is_null() {
            let info = &*ptr;
            let desc = cstr_from_i8(&info.Description);
            let gw = cstr_from_i8(&info.GatewayList.IpAddress.String);
            let ip = cstr_from_i8(&info.IpAddressList.IpAddress.String);

            if !is_virtual_adapter(&desc)
                && !gw.is_empty()
                && gw != "0.0.0.0"
                && !ip.is_empty()
                && ip != "0.0.0.0"
                && ip != "127.0.0.1"
            {
                return (ip, gw);
            }
            ptr = info.Next;
        }

        // Pass 2: any non-virtual adapter with a valid IPv4 address
        ptr = buffer.as_ptr() as *const IP_ADAPTER_INFO;
        while !ptr.is_null() {
            let info = &*ptr;
            let desc = cstr_from_i8(&info.Description);
            let gw = cstr_from_i8(&info.GatewayList.IpAddress.String);
            let ip = cstr_from_i8(&info.IpAddressList.IpAddress.String);

            if !is_virtual_adapter(&desc) && !ip.is_empty() && ip != "0.0.0.0" && ip != "127.0.0.1"
            {
                out_ip = ip;
                if !gw.is_empty() && gw != "0.0.0.0" {
                    out_gw = gw;
                }
                return (out_ip, out_gw);
            }
            ptr = info.Next;
        }
    }

    (out_ip, out_gw)
}

fn get_hostname() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Windows-PC".to_string())
}

fn trigger_repaint(state: &SharedState) {
    let raw = state.lock().map(|s| s.hwnd_raw).unwrap_or(0);
    if raw != 0 {
        unsafe {
            let _ = InvalidateRect(hwnd_from_raw(raw), None, false);
        }
    }
}

fn post_ui_msg(state: &SharedState, msg: u32) {
    let raw = state.lock().map(|s| s.hwnd_raw).unwrap_or(0);
    if raw != 0 {
        unsafe {
            let _ = PostMessageW(hwnd_from_raw(raw), msg, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn spawn_network_threads(state: SharedState, running: RunningFlag) {
    // 1. UDP Discovery Beacon & Responder (Port 52199)
    {
        let st = state.clone();
        let run = running.clone();
        thread::spawn(move || udp_discovery_thread(st, run));
    }

    // 2. TCP Control Server (Port 52200)
    {
        let st = state.clone();
        let run = running.clone();
        thread::spawn(move || control_server_thread(st, run));
    }

    // 3. TCP Turbo File Receiver Server (Port 52201)
    {
        let st = state.clone();
        let run = running.clone();
        thread::spawn(move || file_receiver_server_thread(st, run));
    }
}

fn udp_discovery_thread(state: SharedState, running: RunningFlag) {
    let sock = match Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)) {
        Ok(s) => s,
        Err(_) => return,
    };
    let _ = sock.set_reuse_address(true);
    let _ = sock.set_broadcast(true);
    let _ = sock.set_read_timeout(Some(Duration::from_millis(900)));

    let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, REMOTE_UDP_PORT);
    if sock.bind(&bind_addr.into()).is_err() {
        return;
    }
    let udp: UdpSocket = sock.into();

    let mut recv_buf = [0u8; 512];
    let mut last_beacon = Instant::now() - Duration::from_secs(2);
    let hostname = get_hostname();

    while running.load(Ordering::Relaxed) {
        if last_beacon.elapsed() >= Duration::from_secs(1) {
            last_beacon = Instant::now();
            let (cur_ip, cur_gw) = get_local_network_info();
            let changed = {
                if let Ok(mut s) = state.lock() {
                    if s.local_ip != cur_ip || s.gateway_ip != cur_gw {
                        s.local_ip = cur_ip.clone();
                        s.gateway_ip = cur_gw.clone();
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            };
            if changed {
                trigger_repaint(&state);
            }

            let announce = format!(
                "{}|{}|{}|{}",
                DISCOVER_PONG_MAGIC, hostname, REMOTE_CONTROL_PORT, cur_ip
            );
            let bytes = announce.as_bytes();

            // 1. General Broadcast
            let _ = udp.send_to(bytes, SocketAddrV4::new(Ipv4Addr::BROADCAST, REMOTE_UDP_PORT));

            // 2. Direct send to Default Gateway (Phone IP on Mobile Hotspot)
            if let Ok(gw_ip) = cur_gw.parse::<Ipv4Addr>() {
                if !gw_ip.is_unspecified() {
                    let _ = udp.send_to(bytes, SocketAddrV4::new(gw_ip, REMOTE_UDP_PORT));
                }
            }

            // 3. Subnet Directed Broadcast (e.g. 192.168.43.255)
            if let Ok(local_v4) = cur_ip.parse::<Ipv4Addr>() {
                let o = local_v4.octets();
                let sub_bcast = Ipv4Addr::new(o[0], o[1], o[2], 255);
                let _ = udp.send_to(bytes, SocketAddrV4::new(sub_bcast, REMOTE_UDP_PORT));
            }
        }

        if let Ok((n, src)) = udp.recv_from(&mut recv_buf) {
            let msg = String::from_utf8_lossy(&recv_buf[..n]);
            if msg.contains(DISCOVER_PING_MAGIC) {
                let reply = format!("{}|{}|{}", DISCOVER_PONG_MAGIC, hostname, REMOTE_CONTROL_PORT);
                let _ = udp.send_to(reply.as_bytes(), src);
            }
        }
    }
}

fn set_exclusive_addr(sock: &Socket) {
    let val: i32 = 1;
    unsafe {
        let raw = SOCKET(sock.as_raw_socket() as usize);
        let bytes = std::slice::from_raw_parts(&val as *const i32 as *const u8, std::mem::size_of::<i32>());
        let _ = setsockopt(raw, SOL_SOCKET as i32, SO_EXCLUSIVEADDRUSE, Some(bytes));
    }
}

fn control_server_thread(state: SharedState, running: RunningFlag) {
    let sock = match Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)) {
        Ok(s) => s,
        Err(_) => return,
    };
    set_exclusive_addr(&sock);
    let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, REMOTE_CONTROL_PORT);
    if let Err(e) = sock.bind(&bind_addr.into()) {
        if let Ok(mut s) = state.lock() {
            s.bind_error = Some(format!("Port {} in use ({})", REMOTE_CONTROL_PORT, e.kind()));
        }
        trigger_repaint(&state);
        return;
    }
    if sock.listen(4).is_err() {
        return;
    }
    let listener: TcpListener = sock.into();
    let _ = listener.set_nonblocking(true);

    while running.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, addr)) => {
                let _ = stream.set_nonblocking(false);
                let peer_ip = addr.ip();
                if let Ok(mut s) = state.lock() {
                    s.client_ip = peer_ip.to_string();
                }
                let st = state.clone();
                let run = running.clone();
                thread::spawn(move || handle_control_client(stream, peer_ip, st, run));
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn handle_control_client(mut stream: TcpStream, peer_ip: IpAddr, state: SharedState, running: RunningFlag) {
    let sock_ref = socket2::SockRef::from(&stream);
    let ka = TcpKeepalive::new()
        .with_time(Duration::from_secs(10))
        .with_interval(Duration::from_secs(5));
    let _ = sock_ref.set_tcp_keepalive(&ka);
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(35)));

    let header = match RemoteHeader::read_from(&mut stream) {
        Ok(h) if h.magic == PROTO_MAGIC && h.msg_type == MsgType::AuthReq as u16 => h,
        _ => return,
    };

    if header.payload_len as usize != MsgAuthReq::WIRE_LEN {
        return;
    }
    let mut auth_buf = [0u8; MsgAuthReq::WIRE_LEN];
    if stream.read_exact(&mut auth_buf).is_err() {
        return;
    }
    let auth_req = match MsgAuthReq::decode(&auth_buf) {
        Ok(a) => a,
        Err(_) => return,
    };

    let allowed = if is_device_paired(&auth_req.device_id) {
        true
    } else {
        post_ui_msg(&state, WM_APP_RESTORE);
        let prompt = format!(
            "A new device is requesting to connect to your PC:\n\n\
            Device: {}\n\
            Device ID: {}\n\n\
            Do you want to allow this phone to control your PC and transfer files?\n\
            (Future connections will connect automatically)",
            auth_req.device_name, auth_req.device_id
        );
        let w_msg = HSTRING::from(prompt);
        let w_title = HSTRING::from("I Connect - New Device Request");
        let choice = unsafe {
            MessageBoxW(
                HWND::default(),
                PCWSTR(w_msg.as_ptr()),
                PCWSTR(w_title.as_ptr()),
                MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND,
            )
        };
        if choice == IDYES {
            save_device_to_whitelist(&auth_req.device_id, &auth_req.device_name);
            true
        } else {
            false
        }
    };

    let resp = MsgAuthResp {
        status: if allowed { 1 } else { 0 },
        message: if allowed {
            "Authenticated".to_string()
        } else {
            "Rejected by PC".to_string()
        },
    };
    if send_frame(&mut stream, MsgType::AuthResp, &resp.encode()).is_err() || !allowed {
        return;
    }

    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));

    let my_ctrl_id = if let Ok(mut s) = state.lock() {
        s.active_control_conn_id = s.active_control_conn_id.wrapping_add(1).max(1);
        let id = s.active_control_conn_id;
        s.is_connected = true;
        s.client_ip = peer_ip.to_string();
        s.client_name = auth_req.device_name.clone();
        s.client_id = auth_req.device_id.clone();
        s.paired_count = crate::state::count_paired_devices();
        s.client_file_workers.retain(|(_, w_ip, _)| *w_ip == peer_ip);
        id
    } else {
        0
    };
    post_ui_msg(&state, WM_APP_TRAY_CONNECTED);
    trigger_repaint(&state);

    let mut payload_buf = [0u8; 1024];
    while running.load(Ordering::Relaxed) {
        let hdr = match RemoteHeader::read_from(&mut stream) {
            Ok(h) => h,
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock => {
                continue;
            }
            Err(_) => break,
        };
        if hdr.magic != PROTO_MAGIC {
            continue;
        }
        let plen = hdr.payload_len as usize;
        if plen > payload_buf.len() {
            let mut left = plen;
            while left > 0 {
                let to_read = left.min(payload_buf.len());
                match stream.read(&mut payload_buf[..to_read]) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => left -= n,
                }
            }
            continue;
        }
        if plen > 0 && stream.read_exact(&mut payload_buf[..plen]).is_err() {
            break;
        }

        if let Ok(mut s) = state.lock() {
            s.total_packets_received += 1;
        }

        match MsgType::from_u16(hdr.msg_type) {
            Some(MsgType::MouseMove) => {
                if let Ok(mm) = MsgMouseMove::decode(&payload_buf[..plen]) {
                    simulate_mouse_move(mm.dx, mm.dy);
                }
            }
            Some(MsgType::MouseClick) => {
                if let Ok(mc) = MsgMouseClick::decode(&payload_buf[..plen]) {
                    simulate_mouse_click(mc.button, mc.action);
                }
            }
            Some(MsgType::MouseScroll) => {
                if let Ok(ms) = MsgMouseScroll::decode(&payload_buf[..plen]) {
                    simulate_mouse_scroll(ms.delta_y, ms.delta_x);
                }
            }
            Some(MsgType::KeyEvent) => {
                if let Ok(ke) = MsgKeyEvent::decode(&payload_buf[..plen]) {
                    simulate_key_event(ke.vk_code, ke.action, ke.modifiers);
                }
            }
            Some(MsgType::TextInput) => {
                if let Ok(ti) = MsgTextInput::decode(&payload_buf[..plen]) {
                    simulate_text_input(&ti.text);
                }
            }
            Some(MsgType::MediaCmd) => {
                if let Ok(mc) = MsgMediaCmd::decode(&payload_buf[..plen]) {
                    execute_media_command(mc.action, mc.value);
                }
            }
            Some(MsgType::SysCmd) => {
                if let Ok(sc) = MsgSysCmd::decode(&payload_buf[..plen]) {
                    execute_system_command(sc.cmd);
                }
            }
            Some(MsgType::Heartbeat) => {
                let _ = send_frame(&mut stream, MsgType::Heartbeat, &[]);
            }
            _ => {}
        }
    }

    let was_active = if let Ok(mut s) = state.lock() {
        if s.active_control_conn_id == my_ctrl_id {
            s.is_connected = false;
            s.client_name = "None".to_string();
            s.client_id.clear();
            s.client_file_stream = None;
            s.client_file_workers.clear();
            true
        } else {
            false
        }
    } else {
        false
    };
    if was_active {
        post_ui_msg(&state, WM_APP_TRAY_DISCONNECTED);
        trigger_repaint(&state);
    }
}

fn configure_turbo_socket(stream: &TcpStream) {
    let sock_ref = socket2::SockRef::from(stream);
    let ka = TcpKeepalive::new()
        .with_time(Duration::from_secs(10))
        .with_interval(Duration::from_secs(5));
    let _ = sock_ref.set_tcp_keepalive(&ka);
    // Keep Nagle enabled on bulk file streams so Wi-Fi A-MPDU frames coalesce cleanly.
    let _ = sock_ref.set_tcp_nodelay(false);
    let _ = sock_ref.set_recv_buffer_size(SOCKET_BUFFER_SIZE as usize);
    let _ = sock_ref.set_send_buffer_size(SOCKET_BUFFER_SIZE as usize);
}

struct RangeBatchProgress {
    start: Instant,
    bytes_done: AtomicU64,
    last_ui_ms: AtomicU64,
    finished: AtomicBool,
    preallocated: Mutex<HashSet<u32>>,
}

static INCOMING_BATCHES: OnceLock<Mutex<HashMap<u64, Arc<RangeBatchProgress>>>> = OnceLock::new();
static NEXT_SEND_BATCH_ID: AtomicU64 = AtomicU64::new(1);

fn incoming_batches() -> &'static Mutex<HashMap<u64, Arc<RangeBatchProgress>>> {
    INCOMING_BATCHES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_or_create_batch(batch_id: u64) -> Arc<RangeBatchProgress> {
    let mut map = incoming_batches().lock().unwrap();
    if let Some(b) = map.get(&batch_id) {
        return b.clone();
    }
    if map.len() > 32 {
        map.clear();
    }
    let b = Arc::new(RangeBatchProgress {
        start: Instant::now(),
        bytes_done: AtomicU64::new(0),
        last_ui_ms: AtomicU64::new(0),
        finished: AtomicBool::new(false),
        preallocated: Mutex::new(HashSet::new()),
    });
    map.insert(batch_id, b.clone());
    b
}

fn seek_write_all(file: &File, mut offset: u64, mut data: &[u8]) -> std::io::Result<()> {
    while !data.is_empty() {
        let n = file.seek_write(data, offset)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "seek_write returned 0",
            ));
        }
        offset += n as u64;
        data = &data[n..];
    }
    Ok(())
}

fn file_receiver_server_thread(state: SharedState, running: RunningFlag) {
    let sock = match Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)) {
        Ok(s) => s,
        Err(_) => return,
    };
    set_exclusive_addr(&sock);
    let _ = sock.set_recv_buffer_size(SOCKET_BUFFER_SIZE as usize);
    let _ = sock.set_send_buffer_size(SOCKET_BUFFER_SIZE as usize);

    let bind_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, REMOTE_FILE_PORT);
    if let Err(e) = sock.bind(&bind_addr.into()) {
        if let Ok(mut s) = state.lock() {
            if s.bind_error.is_none() {
                s.bind_error = Some(format!("Port {} in use ({})", REMOTE_FILE_PORT, e.kind()));
            }
        }
        trigger_repaint(&state);
        return;
    }
    if sock.listen(64).is_err() {
        return;
    }
    let listener: TcpListener = sock.into();
    let _ = listener.set_nonblocking(true);

    let save_dir = crate::state::get_save_directory();

    while running.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((file_stream, addr)) => {
                let st = state.clone();
                let run = running.clone();
                let dir = save_dir.clone();
                let peer_ip = addr.ip();
                thread::spawn(move || handle_file_client(file_stream, peer_ip, st, run, dir));
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn handle_file_client(
    mut file_stream: TcpStream,
    peer_ip: IpAddr,
    state: SharedState,
    running: RunningFlag,
    save_dir: PathBuf,
) {
    let _ = file_stream.set_nonblocking(false);
    configure_turbo_socket(&file_stream);

    let mut my_conn_id = 0u64;
    let mut is_pooled_worker = false;
    if let Ok(cloned) = file_stream.try_clone() {
        if let Ok(mut s) = state.lock() {
            s.client_file_conn_id = s.client_file_conn_id.wrapping_add(1);
            my_conn_id = s.client_file_conn_id;
            s.client_file_stream = Some(cloned);
        }
    }

    let mut chunk_buf = vec![0u8; CHUNK_BUFFER_SIZE];

    while running.load(Ordering::Relaxed) {
        let hdr = match RemoteHeader::read_from(&mut file_stream) {
            Ok(h) => h,
            Err(_) => break,
        };
        if hdr.magic != PROTO_MAGIC {
            break;
        }

        match MsgType::from_u16(hdr.msg_type) {
            Some(MsgType::FileChannelHello) => {
                let mut buf = [0u8; MsgFileChannelHello::WIRE_LEN];
                if file_stream.read_exact(&mut buf).is_err() {
                    break;
                }
                if let Ok(hello) = MsgFileChannelHello::decode(&buf) {
                    if hello.role == 1 {
                        is_pooled_worker = true;
                        if let Ok(cloned) = file_stream.try_clone() {
                            if let Ok(mut s) = state.lock() {
                                s.client_file_workers.retain(|(_, w_ip, _)| *w_ip == peer_ip);
                                if s.client_file_workers.len() >= 12 {
                                    s.client_file_workers.remove(0);
                                }
                                s.client_file_workers.push((my_conn_id, peer_ip, cloned));
                            }
                        }
                    }
                }
            }
            Some(MsgType::FileRangeHeader) => {
                let mut buf = [0u8; MsgFileRangeHeader::WIRE_LEN];
                if file_stream.read_exact(&mut buf).is_err() {
                    break;
                }
                let Ok(range) = MsgFileRangeHeader::decode(&buf) else {
                    break;
                };

                let batch = get_or_create_batch(range.batch_id);
                let mut dest_path = save_dir.clone();
                for part in sanitize_relative_path(&range.file_name) {
                    dest_path.push(part);
                }

                // Preallocate file size once per file_index across parallel workers
                {
                    let mut pre = batch.preallocated.lock().unwrap();
                    if pre.insert(range.file_index) {
                        if let Some(parent) = dest_path.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        if let Ok(f) = OpenOptions::new()
                            .create(true)
                            .write(true)
                            .truncate(true)
                            .open(&dest_path)
                        {
                            let _ = f.set_len(range.file_size);
                        }
                    }
                }

                if let Ok(mut s) = state.lock() {
                    s.transfer.is_transferring = true;
                    s.transfer.is_sending = false;
                    s.transfer.total_files_in_batch = range.total_files;
                    s.transfer.current_file_index = range.file_index + 1;
                    s.transfer.current_file_name = range.file_name.clone();
                    s.transfer.current_file_size = range.batch_total_bytes.max(range.file_size);
                }

                let stripe_file = OpenOptions::new().write(true).open(&dest_path).ok();

                let mut remaining = range.chunk_len;
                let mut cur_offset = range.offset;

                while remaining > 0 && running.load(Ordering::Relaxed) {
                    let to_read = (remaining as usize).min(CHUNK_BUFFER_SIZE);
                    let n = match file_stream.read(&mut chunk_buf[..to_read]) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if let Some(ref file) = stripe_file {
                        let _ = seek_write_all(file, cur_offset, &chunk_buf[..n]);
                    }
                    cur_offset += n as u64;
                    remaining -= n as u64;

                    let done = batch.bytes_done.fetch_add(n as u64, Ordering::Relaxed) + n as u64;
                    let now_ms = batch.start.elapsed().as_millis() as u64;
                    let prev_ms = batch.last_ui_ms.load(Ordering::Relaxed);
                    if (now_ms >= prev_ms + 180 || done >= range.batch_total_bytes)
                        && batch
                            .last_ui_ms
                            .compare_exchange(prev_ms, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                            .is_ok()
                    {
                        let elapsed = batch.start.elapsed().as_secs_f64();
                        let mbps = if elapsed > 0.02 {
                            ((done as f64 / (1024.0 * 1024.0)) / elapsed) as f32
                        } else {
                            0.0
                        };
                        if let Ok(mut s) = state.lock() {
                            s.transfer.current_file_transferred = done;
                            s.transfer.current_speed_mbps = mbps;
                        }
                        trigger_repaint(&state);
                    }
                }
                drop(stripe_file);

                let done = batch.bytes_done.load(Ordering::Relaxed);
                if done >= range.batch_total_bytes && !batch.finished.swap(true, Ordering::SeqCst) {
                    if let Ok(mut map) = incoming_batches().lock() {
                        map.remove(&range.batch_id);
                    }
                    if let Ok(mut s) = state.lock() {
                        s.transfer.is_transferring = false;
                        s.transfer.current_speed_mbps = 0.0;
                    }
                    trigger_repaint(&state);
                    post_ui_msg(&state, WM_APP_TRAY_FILE_DONE);
                }
            }
            Some(MsgType::FileBatchHeader) => {
                let mut buf = [0u8; MsgFileBatchHeader::WIRE_LEN];
                if file_stream.read_exact(&mut buf).is_err() {
                    break;
                }
                if let Ok(batch) = MsgFileBatchHeader::decode(&buf) {
                    if let Ok(mut s) = state.lock() {
                        s.transfer.is_transferring = true;
                        s.transfer.is_sending = false;
                        s.transfer.total_files_in_batch = batch.total_files;
                        s.transfer.current_file_index = 0;
                        s.transfer.current_speed_mbps = 0.0;
                    }
                    trigger_repaint(&state);
                }
            }
            Some(MsgType::FileItemHeader) => {
                let mut buf = [0u8; MsgFileItemHeader::WIRE_LEN];
                if file_stream.read_exact(&mut buf).is_err() {
                    break;
                }
                let item = match MsgFileItemHeader::decode(&buf) {
                    Ok(it) => it,
                    Err(_) => break,
                };

                if let Ok(mut s) = state.lock() {
                    s.transfer.current_file_index = item.file_index + 1;
                    s.transfer.current_file_name = item.file_name.clone();
                    s.transfer.current_file_size = item.file_size;
                    s.transfer.current_file_transferred = 0;
                }
                trigger_repaint(&state);

                let mut dest_path = save_dir.clone();
                for part in sanitize_relative_path(&item.file_name) {
                    dest_path.push(part);
                }
                if let Some(parent) = dest_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }

                let mut writer = File::create(&dest_path)
                    .ok()
                    .map(|f| BufWriter::with_capacity(4 * 1024 * 1024, f));

                let mut remaining = item.file_size;
                let mut last_speed_tick = Instant::now();
                let mut bytes_since_speed: u64 = 0;

                while remaining > 0 && running.load(Ordering::Relaxed) {
                    let to_read = (remaining as usize).min(CHUNK_BUFFER_SIZE);
                    let n = match file_stream.read(&mut chunk_buf[..to_read]) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if let Some(ref mut w) = writer {
                        let _ = w.write_all(&chunk_buf[..n]);
                    }
                    remaining -= n as u64;
                    bytes_since_speed += n as u64;

                    let elapsed = last_speed_tick.elapsed().as_secs_f64();
                    let should_update = elapsed >= 0.2 || remaining == 0;
                    if let Ok(mut s) = state.lock() {
                        s.transfer.current_file_transferred += n as u64;
                        if should_update && elapsed > 0.01 {
                            s.transfer.current_speed_mbps =
                                ((bytes_since_speed as f64 / (1024.0 * 1024.0)) / elapsed) as f32;
                            last_speed_tick = Instant::now();
                            bytes_since_speed = 0;
                        }
                    }
                    if should_update {
                        trigger_repaint(&state);
                    }
                }
                if let Some(mut w) = writer {
                    let _ = w.flush();
                }
            }
            Some(MsgType::FileBatchFinish) => {
                if let Ok(mut s) = state.lock() {
                    s.transfer.is_transferring = false;
                    s.transfer.current_speed_mbps = 0.0;
                }
                trigger_repaint(&state);
                post_ui_msg(&state, WM_APP_TRAY_FILE_DONE);
            }
            _ => {}
        }
    }

    if let Ok(mut s) = state.lock() {
        if is_pooled_worker {
            s.client_file_workers.retain(|(id, _, _)| *id != my_conn_id);
        }
        if s.client_file_conn_id == my_conn_id {
            s.client_file_stream = None;
            s.transfer.is_transferring = false;
        }
    }
    trigger_repaint(&state);
}

struct SendFileEntry {
    full_path: PathBuf,
    rel_name: String,
    file_size: u64,
}

fn collect_dir_files(dir: &Path, rel_prefix: &str, out: &mut Vec<SendFileEntry>, total_bytes: &mut u64) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let rel = format!("{}/{}", rel_prefix, name);
        if path.is_dir() {
            collect_dir_files(&path, &rel, out, total_bytes);
        } else if let Ok(meta) = entry.metadata() {
            let sz = meta.len();
            *total_bytes += sz;
            out.push(SendFileEntry {
                full_path: path,
                rel_name: rel,
                file_size: sz,
            });
        }
    }
}

pub fn send_dropped_paths_to_phone(paths: Vec<PathBuf>, state: SharedState) {
    thread::spawn(move || {
        let mut items = Vec::new();
        let mut total_bytes = 0u64;

        for p in paths {
            let Ok(meta) = fs::metadata(&p) else {
                continue;
            };
            let base_name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "item".to_string());
            if meta.is_dir() {
                collect_dir_files(&p, &base_name, &mut items, &mut total_bytes);
            } else {
                let sz = meta.len();
                total_bytes += sz;
                items.push(SendFileEntry {
                    full_path: p,
                    rel_name: base_name,
                    file_size: sz,
                });
            }
        }

        if items.is_empty() {
            return;
        }

        let workers: Vec<TcpStream> = state
            .lock()
            .ok()
            .map(|s| {
                s.client_file_workers
                    .iter()
                    .filter_map(|(_, _, sock)| sock.try_clone().ok())
                    .collect()
            })
            .unwrap_or_default();

        if !workers.is_empty() {
            // 8-Stream Parallel Striped Transfer (PC -> Phone at max Wi-Fi/Hotspot downlink speed)
            let batch_id = NEXT_SEND_BATCH_ID.fetch_add(1, Ordering::Relaxed);
            let total_files = items.len() as u32;
            let stripe_sz = (total_bytes / 32).clamp(1024 * 1024, STRIPE_SIZE);
            let mut queue = VecDeque::new();
            for (idx, item) in items.iter().enumerate() {
                if item.file_size == 0 {
                    queue.push_back((idx as u32, item.full_path.clone(), item.rel_name.clone(), 0u64, 0u64, 0u64, 0u8));
                } else {
                    let mut off = 0u64;
                    while off < item.file_size {
                        let len = (item.file_size - off).min(stripe_sz);
                        queue.push_back((idx as u32, item.full_path.clone(), item.rel_name.clone(), item.file_size, off, len, 0u8));
                        off += len;
                    }
                }
            }

            if let Ok(mut s) = state.lock() {
                s.transfer.is_transferring = true;
                s.transfer.is_sending = true;
                s.transfer.total_files_in_batch = total_files;
                s.transfer.current_file_index = 1;
                s.transfer.current_file_name = items[0].rel_name.clone();
                s.transfer.current_file_size = total_bytes;
                s.transfer.current_file_transferred = 0;
            }
            trigger_repaint(&state);

            let shared_q = Arc::new(Mutex::new(queue));
            let bytes_done = Arc::new(AtomicU64::new(0));
            let last_ui_ms = Arc::new(AtomicU64::new(0));
            let start = Instant::now();

            let mut handles = Vec::with_capacity(workers.len());
            for mut stream in workers {
                configure_turbo_socket(&stream);
                let q = shared_q.clone();
                let st = state.clone();
                let done_ctr = bytes_done.clone();
                let ui_ms = last_ui_ms.clone();

                handles.push(thread::spawn(move || {
                    let mut chunk_buf = vec![0u8; CHUNK_BUFFER_SIZE];
                    loop {
                        let task = {
                            let mut guard = q.lock().unwrap();
                            guard.pop_front()
                        };
                        let Some((fidx, fpath, rel_name, fsize, offset, len, retries)) = task else {
                            break;
                        };
                        let Ok(file) = File::open(&fpath) else {
                            continue;
                        };

                        let hdr = MsgFileRangeHeader::new(
                            batch_id,
                            fidx,
                            total_files,
                            total_bytes,
                            fsize,
                            offset,
                            len,
                            &rel_name,
                        );
                        if send_frame(&mut stream, MsgType::FileRangeHeader, &hdr.encode()).is_err() {
                            if retries < 4 {
                                q.lock().unwrap().push_back((fidx, fpath, rel_name, fsize, offset, len, retries + 1));
                            }
                            break;
                        }

                        let mut cur_off = offset;
                        let mut rem = len;
                        let mut sock_failed = false;
                        while rem > 0 {
                            let to_read = (rem as usize).min(CHUNK_BUFFER_SIZE);
                            let n = match file.seek_read(&mut chunk_buf[..to_read], cur_off) {
                                Ok(0) | Err(_) => break,
                                Ok(n) => n,
                            };
                            if stream.write_all(&chunk_buf[..n]).is_err() {
                                sock_failed = true;
                                break;
                            }
                            cur_off += n as u64;
                            rem -= n as u64;

                            let done = done_ctr.fetch_add(n as u64, Ordering::Relaxed) + n as u64;
                            let now_ms = start.elapsed().as_millis() as u64;
                            let prev_ms = ui_ms.load(Ordering::Relaxed);
                            if (now_ms >= prev_ms + 180 || done >= total_bytes)
                                && ui_ms
                                    .compare_exchange(prev_ms, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                                    .is_ok()
                            {
                                let el = start.elapsed().as_secs_f64();
                                let mbps = if el > 0.02 {
                                    ((done as f64 / (1024.0 * 1024.0)) / el) as f32
                                } else {
                                    0.0
                                };
                                if let Ok(mut s) = st.lock() {
                                    s.transfer.current_file_index = fidx + 1;
                                    s.transfer.current_file_name = rel_name.clone();
                                    s.transfer.current_file_transferred = done;
                                    s.transfer.current_speed_mbps = mbps;
                                }
                                trigger_repaint(&st);
                            }
                        }
                        if sock_failed {
                            if rem > 0 && retries < 4 {
                                q.lock().unwrap().push_back((fidx, fpath, rel_name, fsize, cur_off, rem, retries + 1));
                            }
                            break;
                        }
                    }
                    let _ = stream.flush();
                }));
            }
            for h in handles {
                let _ = h.join();
            }

            if let Ok(mut s) = state.lock() {
                s.transfer.is_transferring = false;
                s.transfer.current_speed_mbps = 0.0;
            }
            trigger_repaint(&state);
            return;
        }

        let stream_opt = state
            .lock()
            .ok()
            .and_then(|s| s.client_file_stream.as_ref().and_then(|f| f.try_clone().ok()));

        let mut file_stream = match stream_opt {
            Some(s) => s,
            None => {
                let msg = HSTRING::from("Cannot send files: Phone file channel is not connected!\nPlease connect the phone app first.");
                let title = HSTRING::from("I Connect - Send Files");
                unsafe {
                    MessageBoxW(
                        HWND::default(),
                        PCWSTR(msg.as_ptr()),
                        PCWSTR(title.as_ptr()),
                        MB_OK | MB_ICONEXCLAMATION | MB_TOPMOST,
                    );
                }
                return;
            }
        };

        configure_turbo_socket(&file_stream);

        let batch = MsgFileBatchHeader {
            total_files: items.len() as u32,
            total_bytes,
        };
        if send_frame(&mut file_stream, MsgType::FileBatchHeader, &batch.encode()).is_err() {
            return;
        }

        if let Ok(mut s) = state.lock() {
            s.transfer.is_transferring = true;
            s.transfer.is_sending = true;
            s.transfer.total_files_in_batch = items.len() as u32;
            s.transfer.current_file_index = 0;
        }
        trigger_repaint(&state);

        let mut chunk_buf = vec![0u8; CHUNK_BUFFER_SIZE];

        for (idx, item) in items.iter().enumerate() {
            let Ok(f) = File::open(&item.full_path) else {
                continue;
            };
            let mut reader = BufReader::with_capacity(2 * 1024 * 1024, f);

            if let Ok(mut s) = state.lock() {
                s.transfer.current_file_index = (idx + 1) as u32;
                s.transfer.current_file_name = item.rel_name.clone();
                s.transfer.current_file_size = item.file_size;
                s.transfer.current_file_transferred = 0;
            }
            trigger_repaint(&state);

            let item_hdr = MsgFileItemHeader::new(idx as u32, item.file_size, &item.rel_name);
            if send_frame(&mut file_stream, MsgType::FileItemHeader, &item_hdr.encode()).is_err() {
                break;
            }

            let mut remaining = item.file_size;
            let mut last_speed_tick = Instant::now();
            let mut bytes_since_speed: u64 = 0;

            while remaining > 0 {
                let to_read = (remaining as usize).min(CHUNK_BUFFER_SIZE);
                let n = match reader.read(&mut chunk_buf[..to_read]) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if file_stream.write_all(&chunk_buf[..n]).is_err() {
                    break;
                }
                remaining -= n as u64;
                bytes_since_speed += n as u64;

                let elapsed = last_speed_tick.elapsed().as_secs_f64();
                let should_update = elapsed >= 0.2 || remaining == 0;
                if let Ok(mut s) = state.lock() {
                    s.transfer.current_file_transferred += n as u64;
                    if should_update && elapsed > 0.01 {
                        s.transfer.current_speed_mbps =
                            ((bytes_since_speed as f64 / (1024.0 * 1024.0)) / elapsed) as f32;
                        last_speed_tick = Instant::now();
                        bytes_since_speed = 0;
                    }
                }
                if should_update {
                    trigger_repaint(&state);
                }
            }
        }

        let _ = send_frame(&mut file_stream, MsgType::FileBatchFinish, &[]);
        let _ = file_stream.flush();

        if let Ok(mut s) = state.lock() {
            s.transfer.is_transferring = false;
            s.transfer.current_speed_mbps = 0.0;
        }
        trigger_repaint(&state);
    });
}
