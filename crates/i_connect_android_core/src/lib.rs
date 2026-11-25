//! Pure Rust Android JNI engine for I Connect.
//! Uses pre-connect 8MB socket buffer negotiation, kernel-space zero-copy
//! `sendfile64` from detached Android ParcelFileDescriptors, and 4-stream
//! parallel byte-range striping to saturate Wi-Fi / Hotspot / USB links.

use i_connect_proto::{
    sanitize_relative_path, send_frame, CHUNK_BUFFER_SIZE, DISCOVER_PING_MAGIC,
    DISCOVER_PONG_MAGIC, PARALLEL_FILE_STREAMS, PROTO_MAGIC, REMOTE_UDP_PORT, SOCKET_BUFFER_SIZE,
    STRIPE_SIZE,
};
use i_connect_proto::{
    MsgAuthReq, MsgAuthResp, MsgFileBatchHeader, MsgFileChannelHello, MsgFileItemHeader,
    MsgFileRangeHeader, MsgKeyEvent, MsgMediaCmd, MsgMouseClick, MsgMouseMove, MsgMouseScroll,
    MsgSysCmd, MsgTextInput, MsgType, RemoteHeader,
};
use jni::objects::{JClass, JIntArray, JLongArray, JObject, JObjectArray, JString, JValue};
use jni::sys::{jboolean, jint, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use log::{error, info};
use socket2::{Domain, Protocol, Socket, TcpKeepalive, Type};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd};
#[cfg(not(unix))]
use std::os::windows::fs::FileExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

struct CoreState {
    control_sock: Mutex<Option<TcpStream>>,
    file_sock: Mutex<Option<TcpStream>>,
    worker_socks: Mutex<Vec<TcpStream>>,
    device_id: Mutex<String>,
    device_name: Mutex<String>,
}

static CORE: OnceLock<CoreState> = OnceLock::new();
static NEXT_BATCH_ID: AtomicU64 = AtomicU64::new(1);
static ACTIVE_WORKERS: AtomicU32 = AtomicU32::new(0);

fn core() -> &'static CoreState {
    CORE.get_or_init(|| CoreState {
        control_sock: Mutex::new(None),
        file_sock: Mutex::new(None),
        worker_socks: Mutex::new(Vec::new()),
        device_id: Mutex::new("android-device-default".to_string()),
        device_name: Mutex::new("Android Phone".to_string()),
    })
}

struct WorkerGuard {
    counted: bool,
}

impl WorkerGuard {
    fn activate(&mut self) {
        if !self.counted {
            self.counted = true;
            ACTIVE_WORKERS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if self.counted {
            ACTIVE_WORKERS.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

struct IncomingBatchProgress {
    start: Instant,
    bytes_done: AtomicU64,
    last_ui_ms: AtomicU64,
    finished: AtomicBool,
    preallocated: Mutex<HashSet<u32>>,
}

static INCOMING_BATCHES: OnceLock<Mutex<HashMap<u64, Arc<IncomingBatchProgress>>>> =
    OnceLock::new();

fn incoming_batches() -> &'static Mutex<HashMap<u64, Arc<IncomingBatchProgress>>> {
    INCOMING_BATCHES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_or_create_incoming_batch(batch_id: u64) -> Arc<IncomingBatchProgress> {
    let mut map = incoming_batches().lock().unwrap();
    if let Some(b) = map.get(&batch_id) {
        return b.clone();
    }
    if map.len() > 32 {
        map.clear();
    }
    let b = Arc::new(IncomingBatchProgress {
        start: Instant::now(),
        bytes_done: AtomicU64::new(0),
        last_ui_ms: AtomicU64::new(0),
        finished: AtomicBool::new(false),
        preallocated: Mutex::new(HashSet::new()),
    });
    map.insert(batch_id, b.clone());
    b
}

fn send_control_packet(msg_type: MsgType, payload: &[u8]) -> i32 {
    let Ok(mut guard) = core().control_sock.lock() else {
        return -1;
    };
    let Some(ref mut stream) = *guard else {
        return -1;
    };
    if send_frame(stream, msg_type, payload).is_err() {
        *guard = None;
        return -1;
    }
    0
}

/// Opens a TCP bulk-transfer socket with 8MB buffers configured BEFORE connect()
/// so the kernel negotiates full TCP Window Scaling during the SYN handshake.
fn connect_turbo_file_socket(ip: Ipv4Addr, port: u16) -> Option<TcpStream> {
    let sock = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).ok()?;
    let ka = TcpKeepalive::new()
        .with_time(Duration::from_secs(10))
        .with_interval(Duration::from_secs(5));
    let _ = sock.set_tcp_keepalive(&ka);
    let _ = sock.set_send_buffer_size(SOCKET_BUFFER_SIZE as usize);
    let _ = sock.set_recv_buffer_size(SOCKET_BUFFER_SIZE as usize);
    let _ = sock.set_tcp_nodelay(false);
    let addr = SocketAddr::V4(SocketAddrV4::new(ip, port));
    sock.connect_timeout(&addr.into(), Duration::from_secs(5)).ok()?;
    let stream: TcpStream = sock.into();
    let _ = stream.set_nodelay(false);
    Some(stream)
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_init(
    mut env: JNIEnv,
    _class: JClass,
    jid: JString,
    jname: JString,
) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("IConnect_Rust"),
    );

    if let Ok(id) = env.get_string(&jid) {
        if let Ok(mut g) = core().device_id.lock() {
            *g = id.into();
        }
    }
    if let Ok(name) = env.get_string(&jname) {
        if let Ok(mut g) = core().device_name.lock() {
            *g = name.into();
        }
    }
    info!("I Connect Rust NativeCore initialized (HyperLink 4-stream + sendfile64)");
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_discoverServer(
    env: JNIEnv,
    _class: JClass,
) -> jstring {
    let found_ip = (|| -> Option<String> {
        let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).ok()?;
        let _ = sock.set_broadcast(true);
        let _ = sock.set_reuse_address(true);
        let _ = sock.set_read_timeout(Some(Duration::from_millis(1200)));
        let _ = sock.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0).into());
        let udp: UdpSocket = sock.into();

        let ping = DISCOVER_PING_MAGIC.as_bytes();
        let _ = udp.send_to(ping, SocketAddrV4::new(Ipv4Addr::BROADCAST, REMOTE_UDP_PORT));
        let _ = udp.send_to(
            ping,
            SocketAddrV4::new(Ipv4Addr::new(192, 168, 43, 255), REMOTE_UDP_PORT),
        );
        for host in 2..=40u8 {
            let _ = udp.send_to(
                ping,
                SocketAddrV4::new(Ipv4Addr::new(192, 168, 43, host), REMOTE_UDP_PORT),
            );
        }

        let mut buf = [0u8; 256];
        if let Ok((n, src)) = udp.recv_from(&mut buf) {
            let text = String::from_utf8_lossy(&buf[..n]);
            if text.contains(DISCOVER_PONG_MAGIC) {
                let ip = src.ip().to_string();
                info!("Discovered PC via UDP at {}: {}", ip, text);
                return Some(ip);
            }
        }
        None
    })()
    .unwrap_or_default();

    env.new_string(found_ip)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_connectControl(
    mut env: JNIEnv,
    _class: JClass,
    jip: JString,
    port: jint,
) -> jint {
    let Ok(ip_jstr) = env.get_string(&jip) else {
        return -1;
    };
    let ip_str: String = ip_jstr.into();
    let Ok(ip) = ip_str.parse::<Ipv4Addr>() else {
        return -2;
    };

    if let Ok(mut g) = core().control_sock.lock() {
        *g = None;
    }

    let addr = SocketAddr::V4(SocketAddrV4::new(ip, port as u16));
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(4)) {
        Ok(s) => s,
        Err(e) => {
            error!("Control connect failed: {:?}", e);
            return -2;
        }
    };

    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(Duration::from_secs(4)));
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));

    let device_id = core().device_id.lock().map(|g| g.clone()).unwrap_or_default();
    let device_name = core()
        .device_name
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    let req = MsgAuthReq {
        device_id,
        device_name,
    };
    if send_frame(&mut stream, MsgType::AuthReq, &req.encode()).is_err() {
        return -3;
    }

    let hdr = match RemoteHeader::read_from(&mut stream) {
        Ok(h) if h.magic == PROTO_MAGIC && h.msg_type == MsgType::AuthResp as u16 => h,
        _ => return -3,
    };
    if hdr.payload_len as usize != MsgAuthResp::WIRE_LEN {
        return -3;
    }
    let mut resp_buf = [0u8; MsgAuthResp::WIRE_LEN];
    if stream.read_exact(&mut resp_buf).is_err() {
        return -3;
    }
    let Ok(resp) = MsgAuthResp::decode(&resp_buf) else {
        return -3;
    };
    if resp.status != 1 {
        return 0;
    }

    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    if let Ok(mut g) = core().control_sock.lock() {
        *g = Some(stream);
    }
    info!("Connected to PC control channel at {}:{}", ip_str, port);
    1
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_disconnect(_env: JNIEnv, _class: JClass) {
    if let Ok(mut g) = core().control_sock.lock() {
        if let Some(s) = g.take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
    if let Ok(mut g) = core().file_sock.lock() {
        if let Some(s) = g.take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
    if let Ok(mut workers) = core().worker_socks.lock() {
        for s in workers.drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_isConnected(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    let connected = core()
        .control_sock
        .lock()
        .map(|g| g.is_some())
        .unwrap_or(false);
    if connected {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_sendMouseMove(
    _env: JNIEnv,
    _class: JClass,
    dx: jint,
    dy: jint,
) {
    let mm = MsgMouseMove {
        dx: dx.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        dy: dy.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        smooth: 1,
    };
    send_control_packet(MsgType::MouseMove, &mm.encode());
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_sendMouseClick(
    _env: JNIEnv,
    _class: JClass,
    button: jint,
    action: jint,
) {
    let mc = MsgMouseClick {
        button: button as u8,
        action: action as u8,
    };
    send_control_packet(MsgType::MouseClick, &mc.encode());
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_sendMouseScroll(
    _env: JNIEnv,
    _class: JClass,
    delta_y: jint,
    delta_x: jint,
) {
    let ms = MsgMouseScroll {
        delta_y: delta_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        delta_x: delta_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
    };
    send_control_packet(MsgType::MouseScroll, &ms.encode());
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_sendKeyEvent(
    _env: JNIEnv,
    _class: JClass,
    vk_code: jint,
    action: jint,
    modifiers: jint,
) {
    let ke = MsgKeyEvent {
        vk_code: vk_code as u16,
        action: action as u8,
        modifiers: modifiers as u8,
    };
    send_control_packet(MsgType::KeyEvent, &ke.encode());
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_sendTextInput(
    mut env: JNIEnv,
    _class: JClass,
    jtext: JString,
) {
    let Ok(s) = env.get_string(&jtext) else {
        return;
    };
    let text: String = s.into();
    let ti = MsgTextInput::new(&text);
    send_control_packet(MsgType::TextInput, &ti.encode());
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_sendMediaCmd(
    _env: JNIEnv,
    _class: JClass,
    action: jint,
    value: jint,
) {
    let mc = MsgMediaCmd {
        action: action as u8,
        value: value as u8,
    };
    send_control_packet(MsgType::MediaCmd, &mc.encode());
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_sendSysCmd(
    _env: JNIEnv,
    _class: JClass,
    cmd: jint,
) {
    let sc = MsgSysCmd { cmd: cmd as u8 };
    send_control_packet(MsgType::SysCmd, &sc.encode());
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_connectFileChannel(
    mut env: JNIEnv,
    _class: JClass,
    jip: JString,
    port: jint,
) -> jint {
    let Ok(ip_jstr) = env.get_string(&jip) else {
        return -1;
    };
    let ip_str: String = ip_jstr.into();
    let Ok(ip) = ip_str.parse::<Ipv4Addr>() else {
        return -2;
    };

    if let Ok(mut g) = core().file_sock.lock() {
        *g = None;
    }

    let Some(stream) = connect_turbo_file_socket(ip, port as u16) else {
        return -2;
    };

    if let Ok(mut g) = core().file_sock.lock() {
        *g = Some(stream);
    }
    1
}

fn invoke_progress(
    env: &mut JNIEnv,
    callback: &JObject,
    file_idx: i32,
    total_files: i32,
    name: &str,
    transferred: i64,
    total_bytes: i64,
    mbps: f32,
) {
    if callback.is_null() {
        return;
    }
    if let Ok(jname) = env.new_string(name) {
        let _ = env.call_method(
            callback,
            "onFileProgress",
            "(IILjava/lang/String;JJF)V",
            &[
                JValue::Int(file_idx),
                JValue::Int(total_files),
                JValue::Object(&jname),
                JValue::Long(transferred),
                JValue::Long(total_bytes),
                JValue::Float(mbps),
            ],
        );
        let _ = env.delete_local_ref(jname);
    }
}

fn invoke_batch_finish(env: &mut JNIEnv, callback: &JObject, total_files: i32) {
    if callback.is_null() {
        return;
    }
    let _ = env.call_method(callback, "onBatchFinished", "(I)V", &[JValue::Int(total_files)]);
}

struct SendFdEntry {
    file: Arc<File>,
    rel_name: String,
    file_size: u64,
}

struct StripeTask {
    file_index: u32,
    entry: Arc<SendFdEntry>,
    offset: u64,
    len: u64,
}

#[cfg(unix)]
fn stream_fd_stripe(
    stream: &mut TcpStream,
    file: &File,
    mut offset: u64,
    mut remaining: u64,
    bytes_done: &AtomicU64,
) -> bool {
    let sock_fd = stream.as_raw_fd();
    let in_fd = file.as_raw_fd();
    let mut use_sendfile = true;
    let mut fallback_buf: Vec<u8> = Vec::new();

    while remaining > 0 {
        let step = (remaining as usize).min(CHUNK_BUFFER_SIZE);
        if use_sendfile {
            let mut off64 = offset as libc::off64_t;
            let n = unsafe { libc::sendfile64(sock_fd, in_fd, &mut off64, step) };
            if n > 0 {
                let sent = n as u64;
                offset += sent;
                remaining -= sent;
                bytes_done.fetch_add(sent, Ordering::Relaxed);
                continue;
            }
            if n == 0 {
                return false;
            }
            let err = std::io::Error::last_os_error();
            let raw = err.raw_os_error().unwrap_or(0);
            if raw == libc::EINTR || raw == libc::EAGAIN {
                continue;
            }
            if raw == libc::EINVAL || raw == libc::ESPIPE || raw == libc::ENOSYS {
                use_sendfile = false;
            } else {
                error!("sendfile64 error: {:?}", err);
                return false;
            }
        }

        if fallback_buf.is_empty() {
            fallback_buf.resize(CHUNK_BUFFER_SIZE, 0);
        }
        let n = match file.read_at(&mut fallback_buf[..step], offset) {
            Ok(0) | Err(_) => return false,
            Ok(n) => n,
        };
        if stream.write_all(&fallback_buf[..n]).is_err() {
            return false;
        }
        offset += n as u64;
        remaining -= n as u64;
        bytes_done.fetch_add(n as u64, Ordering::Relaxed);
    }
    true
}

#[cfg(not(unix))]
fn stream_fd_stripe(
    _stream: &mut TcpStream,
    _file: &File,
    _offset: u64,
    _remaining: u64,
    _bytes_done: &AtomicU64,
) -> bool {
    false
}

#[allow(unused_mut)]
fn write_all_at_offset(file: &File, mut buf: &[u8], mut offset: u64) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        file.write_all_at(buf, offset)
    }
    #[cfg(not(unix))]
    {
        while !buf.is_empty() {
            let n = file.seek_write(buf, offset)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "failed to write file chunk",
                ));
            }
            buf = &buf[n..];
            offset += n as u64;
        }
        Ok(())
    }
}

/// Zero-copy 4-stream parallel file transfer directly from detached Android ParcelFileDescriptors.
#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_sendFilesFromFds(
    mut env: JNIEnv,
    _class: JClass,
    jfds: JIntArray,
    jrelnames: JObjectArray,
    jsizes: JLongArray,
    jip: JString,
    port: jint,
    jcallback: JObject,
) -> jint {
    let Ok(ip_jstr) = env.get_string(&jip) else {
        return -1;
    };
    let ip_str: String = ip_jstr.into();
    let Ok(server_ip) = ip_str.parse::<Ipv4Addr>() else {
        return -2;
    };

    let count = env.get_array_length(&jfds).unwrap_or(0) as usize;
    if count == 0 {
        return 0;
    }

    let mut raw_fds = vec![0i32; count];
    if env.get_int_array_region(&jfds, 0, &mut raw_fds).is_err() {
        return -1;
    }

    let mut raw_sizes = vec![0i64; count];
    if !jsizes.is_null() {
        let _ = env.get_long_array_region(&jsizes, 0, &mut raw_sizes);
    }

    let mut entries: Vec<Arc<SendFdEntry>> = Vec::with_capacity(count);
    let mut total_bytes: u64 = 0;

    for i in 0..count {
        let fd = raw_fds[i];
        if fd < 0 {
            continue;
        }

        let mut rel_name = format!("file_{}", i);
        if let Ok(obj) = env.get_object_array_element(&jrelnames, i as i32) {
            if !obj.is_null() {
                let jrel = JString::from(obj);
                if let Some(s) = env.get_string(&jrel).ok().map(String::from) {
                    if !s.is_empty() {
                        rel_name = s;
                    }
                }
            }
        }
        rel_name = rel_name.replace('\\', "/");

        #[cfg(not(unix))]
        let opened: Option<(File, u64)> = {
            let _ = (&fd, &raw_sizes);
            None
        };
        #[cfg(unix)]
        let opened: Option<(File, u64)> = {
            // Take immediate ownership of the detached fd so it is always closed on drop
            let file = unsafe { File::from_raw_fd(fd) };
            let mut fsize = if raw_sizes[i] > 0 {
                raw_sizes[i] as u64
            } else {
                file.metadata().map(|m| m.len()).unwrap_or(0)
            };
            if fsize == 0 {
                let end_pos = unsafe { libc::lseek64(fd, 0, libc::SEEK_END) };
                if end_pos > 0 {
                    fsize = end_pos as u64;
                    unsafe {
                        libc::lseek64(fd, 0, libc::SEEK_SET);
                    }
                }
            }
            Some((file, fsize))
        };
        let Some((file, fsize)) = opened else {
            return -1;
        };

        total_bytes += fsize;
        entries.push(Arc::new(SendFdEntry {
            file: Arc::new(file),
            rel_name,
            file_size: fsize,
        }));
    }

    if entries.is_empty() {
        return 0;
    }

    let total_files = entries.len() as u32;
    let batch_id = NEXT_BATCH_ID.fetch_add(1, Ordering::Relaxed);
    let mut queue = VecDeque::new();

    for (idx, entry) in entries.iter().enumerate() {
        if entry.file_size == 0 {
            queue.push_back(StripeTask {
                file_index: idx as u32,
                entry: entry.clone(),
                offset: 0,
                len: 0,
            });
        } else {
            let mut off = 0u64;
            while off < entry.file_size {
                let len = (entry.file_size - off).min(STRIPE_SIZE);
                queue.push_back(StripeTask {
                    file_index: idx as u32,
                    entry: entry.clone(),
                    offset: off,
                    len,
                });
                off += len;
            }
        }
    }

    let stream_count = if total_bytes < 512 * 1024 || queue.len() == 1 && total_bytes < STRIPE_SIZE {
        1
    } else {
        PARALLEL_FILE_STREAMS.min(queue.len().max(1))
    };

    let mut sockets = Vec::with_capacity(stream_count);
    for _ in 0..stream_count {
        if let Some(s) = connect_turbo_file_socket(server_ip, port as u16) {
            sockets.push(s);
        }
    }
    if sockets.is_empty() {
        return -2;
    }

    let shared_q = Arc::new(Mutex::new(queue));
    let bytes_done = Arc::new(AtomicU64::new(0));
    let active_idx = Arc::new(AtomicU32::new(1));
    let active_name = Arc::new(Mutex::new(entries[0].rel_name.clone()));
    let workers_finished = Arc::new(AtomicBool::new(false));
    let active_workers = Arc::new(AtomicU32::new(sockets.len() as u32));

    let start = Instant::now();
    let mut handles = Vec::with_capacity(sockets.len());

    for mut stream in sockets {
        let q = shared_q.clone();
        let done_ctr = bytes_done.clone();
        let a_idx = active_idx.clone();
        let a_name = active_name.clone();
        let w_left = active_workers.clone();
        let fin_flag = workers_finished.clone();

        handles.push(thread::spawn(move || {
            loop {
                let task = {
                    let mut guard = q.lock().unwrap();
                    guard.pop_front()
                };
                let Some(t) = task else {
                    break;
                };

                a_idx.store(t.file_index + 1, Ordering::Relaxed);
                if let Ok(mut n) = a_name.lock() {
                    *n = t.entry.rel_name.clone();
                }

                let hdr = MsgFileRangeHeader::new(
                    batch_id,
                    t.file_index,
                    total_files,
                    total_bytes,
                    t.entry.file_size,
                    t.offset,
                    t.len,
                    &t.entry.rel_name,
                );
                if send_frame(&mut stream, MsgType::FileRangeHeader, &hdr.encode()).is_err() {
                    break;
                }

                if t.len > 0
                    && !stream_fd_stripe(&mut stream, &t.entry.file, t.offset, t.len, &done_ctr)
                {
                    break;
                }
            }
            let _ = stream.flush();
            if w_left.fetch_sub(1, Ordering::AcqRel) == 1 {
                fin_flag.store(true, Ordering::Release);
            }
        }));
    }

    // Calling Java thread pumps throttled progress callbacks cleanly using its own valid JNIEnv
    while !workers_finished.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(160));
        let done = bytes_done.load(Ordering::Relaxed);
        let el = start.elapsed().as_secs_f64();
        let mbps = if el > 0.03 {
            ((done as f64 / (1024.0 * 1024.0)) / el) as f32
        } else {
            0.0
        };
        let idx = active_idx.load(Ordering::Relaxed) as i32;
        let name = active_name
            .lock()
            .map(|g| g.clone())
            .unwrap_or_else(|_| "Transferring".to_string());
        invoke_progress(
            &mut env,
            &jcallback,
            idx,
            total_files as i32,
            &name,
            done as i64,
            total_bytes.max(1) as i64,
            mbps,
        );
    }

    for h in handles {
        let _ = h.join();
    }

    let final_done = bytes_done.load(Ordering::Relaxed);
    let el = start.elapsed().as_secs_f64();
    let final_mbps = if el > 0.02 {
        ((final_done as f64 / (1024.0 * 1024.0)) / el) as f32
    } else {
        0.0
    };
    invoke_progress(
        &mut env,
        &jcallback,
        total_files as i32,
        total_files as i32,
        &entries.last().unwrap().rel_name,
        total_bytes as i64,
        total_bytes.max(1) as i64,
        final_mbps,
    );
    invoke_batch_finish(&mut env, &jcallback, total_files as i32);

    info!(
        "HyperLink 4-stream batch complete: {} bytes in {:.2}s ({:.2} MB/s)",
        final_done, el, final_mbps
    );
    total_files as i32
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_sendFilesBatchWithNames(
    mut env: JNIEnv,
    _class: JClass,
    jpaths: JObjectArray,
    jrelnames: JObjectArray,
    jcallback: JObject,
) -> jint {
    let stream_opt = core()
        .file_sock
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|s| s.try_clone().ok()));
    let mut stream = match stream_opt {
        Some(s) => s,
        None => return -1,
    };

    let num_files = env.get_array_length(&jpaths).unwrap_or(0);
    if num_files <= 0 {
        return 0;
    }

    let mut entries: Vec<(PathBuf, String, u64)> = Vec::with_capacity(num_files as usize);
    let mut total_bytes: u64 = 0;

    for i in 0..num_files {
        let Ok(obj) = env.get_object_array_element(&jpaths, i) else {
            continue;
        };
        let jpath = JString::from(obj);
        let Ok(path_jstr) = env.get_string(&jpath) else {
            continue;
        };
        let path_str: String = path_jstr.into();
        let path_buf = PathBuf::from(&path_str);

        let mut rel_name = path_buf
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());

        if !jrelnames.is_null() {
            if let Ok(rel_obj) = env.get_object_array_element(&jrelnames, i) {
                if !rel_obj.is_null() {
                    let jrel = JString::from(rel_obj);
                    let opt_rel = env.get_string(&jrel).ok().map(String::from);
                    if let Some(s) = opt_rel {
                        if !s.is_empty() {
                            rel_name = s;
                        }
                    }
                }
            }
        }
        rel_name = rel_name.replace('\\', "/");

        let size = fs::metadata(&path_buf).map(|m| m.len()).unwrap_or(0);
        total_bytes += size;
        entries.push((path_buf, rel_name, size));
    }

    let batch = MsgFileBatchHeader {
        total_files: entries.len() as u32,
        total_bytes,
    };
    if send_frame(&mut stream, MsgType::FileBatchHeader, &batch.encode()).is_err() {
        return -1;
    }

    let mut chunk_buf = vec![0u8; CHUNK_BUFFER_SIZE];
    let total_count = entries.len() as i32;

    for (idx, (path, rel_name, file_size)) in entries.iter().enumerate() {
        let Ok(f) = File::open(path) else {
            continue;
        };
        let mut reader = BufReader::with_capacity(4 * 1024 * 1024, f);

        let item_hdr = MsgFileItemHeader::new(idx as u32, *file_size, rel_name);
        if send_frame(&mut stream, MsgType::FileItemHeader, &item_hdr.encode()).is_err() {
            break;
        }

        let mut remaining = *file_size;
        let mut transferred = 0u64;
        let start = Instant::now();
        let mut last_cb = start;

        while remaining > 0 {
            let to_read = (remaining as usize).min(CHUNK_BUFFER_SIZE);
            let n = match reader.read(&mut chunk_buf[..to_read]) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if stream.write_all(&chunk_buf[..n]).is_err() {
                break;
            }
            remaining -= n as u64;
            transferred += n as u64;

            if last_cb.elapsed() >= Duration::from_millis(200) || remaining == 0 {
                let total_sec = start.elapsed().as_secs_f64();
                let mbps = if total_sec > 0.05 {
                    ((transferred as f64 / (1024.0 * 1024.0)) / total_sec) as f32
                } else {
                    0.0
                };
                invoke_progress(
                    &mut env,
                    &jcallback,
                    (idx + 1) as i32,
                    total_count,
                    rel_name,
                    transferred as i64,
                    *file_size as i64,
                    mbps,
                );
                last_cb = Instant::now();
            }
        }
    }

    let _ = send_frame(&mut stream, MsgType::FileBatchFinish, &[]);
    let _ = stream.flush();
    invoke_batch_finish(&mut env, &jcallback, total_count);
    total_count
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_sendFilesBatch(
    env: JNIEnv,
    class: JClass,
    jpaths: JObjectArray,
    jcallback: JObject,
) -> jint {
    let null_names = JObjectArray::from(JObject::null());
    Java_com_iconnect_app_NativeCore_sendFilesBatchWithNames(env, class, jpaths, null_names, jcallback)
}

fn run_incoming_stream_loop(
    mut env: JNIEnv,
    mut stream: TcpStream,
    save_dir: PathBuf,
    jcallback: JObject,
) {
    let mut chunk_buf = vec![0u8; CHUNK_BUFFER_SIZE];
    let mut batch_total_files: i32 = 1;

    loop {
        let hdr = match RemoteHeader::read_from(&mut stream) {
            Ok(h) => h,
            Err(_) => break,
        };
        if hdr.magic != PROTO_MAGIC {
            break;
        }

        match MsgType::from_u16(hdr.msg_type) {
            Some(MsgType::FileRangeHeader) => {
                let mut buf = [0u8; MsgFileRangeHeader::WIRE_LEN];
                if stream.read_exact(&mut buf).is_err() {
                    break;
                }
                let Ok(range) = MsgFileRangeHeader::decode(&buf) else {
                    break;
                };
                let batch = get_or_create_incoming_batch(range.batch_id);
                let mut out_path = save_dir.clone();
                for part in sanitize_relative_path(&range.file_name) {
                    out_path.push(part);
                }

                {
                    let mut pre = batch.preallocated.lock().unwrap();
                    if pre.insert(range.file_index) {
                        if let Some(parent) = out_path.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        if let Ok(f) = OpenOptions::new()
                            .create(true)
                            .write(true)
                            .truncate(true)
                            .open(&out_path)
                        {
                            let _ = f.set_len(range.file_size);
                        }
                    }
                }

                let stripe_file = OpenOptions::new().write(true).open(&out_path).ok();

                let mut remaining = range.chunk_len;
                let mut cur_offset = range.offset;
                while remaining > 0 {
                    let to_read = (remaining as usize).min(CHUNK_BUFFER_SIZE);
                    let n = match stream.read(&mut chunk_buf[..to_read]) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if let Some(ref f) = stripe_file {
                        let _ = write_all_at_offset(f, &chunk_buf[..n], cur_offset);
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
                        let el = batch.start.elapsed().as_secs_f64();
                        let mbps = if el > 0.02 {
                            ((done as f64 / (1024.0 * 1024.0)) / el) as f32
                        } else {
                            0.0
                        };
                        invoke_progress(
                            &mut env,
                            &jcallback,
                            (range.file_index + 1) as i32,
                            range.total_files as i32,
                            &range.file_name,
                            done as i64,
                            range.batch_total_bytes.max(1) as i64,
                            mbps,
                        );
                    }
                }
                drop(stripe_file);

                let done = batch.bytes_done.load(Ordering::Relaxed);
                if done >= range.batch_total_bytes && !batch.finished.swap(true, Ordering::SeqCst) {
                    if let Ok(mut map) = incoming_batches().lock() {
                        map.remove(&range.batch_id);
                    }
                    invoke_batch_finish(&mut env, &jcallback, range.total_files as i32);
                }
            }
            Some(MsgType::FileBatchHeader) => {
                let mut buf = [0u8; MsgFileBatchHeader::WIRE_LEN];
                if stream.read_exact(&mut buf).is_err() {
                    break;
                }
                if let Ok(batch) = MsgFileBatchHeader::decode(&buf) {
                    batch_total_files = batch.total_files.max(1) as i32;
                }
            }
            Some(MsgType::FileItemHeader) => {
                let mut buf = [0u8; MsgFileItemHeader::WIRE_LEN];
                if stream.read_exact(&mut buf).is_err() {
                    break;
                }
                let Ok(item) = MsgFileItemHeader::decode(&buf) else {
                    break;
                };

                let mut out_path = save_dir.clone();
                for part in sanitize_relative_path(&item.file_name) {
                    out_path.push(part);
                }
                if let Some(parent) = out_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }

                let mut writer = File::create(&out_path)
                    .ok()
                    .map(|f| BufWriter::with_capacity(4 * 1024 * 1024, f));

                let mut remaining = item.file_size;
                let mut received = 0u64;
                let start = Instant::now();
                let mut last_cb = start;

                while remaining > 0 {
                    let to_read = (remaining as usize).min(CHUNK_BUFFER_SIZE);
                    let n = match stream.read(&mut chunk_buf[..to_read]) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if let Some(ref mut w) = writer {
                        let _ = w.write_all(&chunk_buf[..n]);
                    }
                    remaining -= n as u64;
                    received += n as u64;

                    if last_cb.elapsed() >= Duration::from_millis(200) || remaining == 0 {
                        let total_sec = start.elapsed().as_secs_f64();
                        let mbps = if total_sec > 0.05 {
                            ((received as f64 / (1024.0 * 1024.0)) / total_sec) as f32
                        } else {
                            0.0
                        };
                        invoke_progress(
                            &mut env,
                            &jcallback,
                            (item.file_index + 1) as i32,
                            batch_total_files,
                            &item.file_name,
                            received as i64,
                            item.file_size as i64,
                            mbps,
                        );
                        last_cb = Instant::now();
                    }
                }
                if let Some(mut w) = writer {
                    let _ = w.flush();
                }
            }
            Some(MsgType::FileBatchFinish) => {
                invoke_batch_finish(&mut env, &jcallback, batch_total_files);
            }
            _ => {}
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_closeFileWorkers(
    _env: JNIEnv,
    _class: JClass,
) {
    if let Ok(mut g) = core().file_sock.lock() {
        if let Some(s) = g.take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
    if let Ok(mut workers) = core().worker_socks.lock() {
        for s in workers.drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_getActiveWorkerCount(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    ACTIVE_WORKERS.load(Ordering::Relaxed) as jint
}

/// Opens a pooled incoming worker socket to PC port 52201, registers via FileChannelHello,
/// and receives parallel PC -> Phone stripes at max wire speed.
#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_runIncomingWorker(
    mut env: JNIEnv,
    _class: JClass,
    jip: JString,
    port: jint,
    jsave_dir: JString,
    jcallback: JObject,
) {
    let Ok(ip_jstr) = env.get_string(&jip) else {
        return;
    };
    let ip_str: String = ip_jstr.into();
    let Ok(ip) = ip_str.parse::<Ipv4Addr>() else {
        return;
    };
    let Ok(dir_jstr) = env.get_string(&jsave_dir) else {
        return;
    };
    let save_dir = PathBuf::from(String::from(dir_jstr));

    let Some(mut stream) = connect_turbo_file_socket(ip, port as u16) else {
        return;
    };

    let device_id = core().device_id.lock().map(|g| g.clone()).unwrap_or_default();
    let hello = MsgFileChannelHello { role: 1, device_id };
    if send_frame(&mut stream, MsgType::FileChannelHello, &hello.encode()).is_err() {
        return;
    }

    let mut guard = WorkerGuard { counted: false };
    guard.activate();

    if let Ok(cloned) = stream.try_clone() {
        if let Ok(mut workers) = core().worker_socks.lock() {
            workers.push(cloned);
        }
    }

    run_incoming_stream_loop(env, stream, save_dir, jcallback);
}

#[no_mangle]
pub extern "system" fn Java_com_iconnect_app_NativeCore_listenIncomingFiles(
    mut env: JNIEnv,
    _class: JClass,
    jsave_dir: JString,
    jcallback: JObject,
) {
    let Ok(dir_jstr) = env.get_string(&jsave_dir) else {
        return;
    };
    let save_dir = PathBuf::from(String::from(dir_jstr));

    let stream_opt = core()
        .file_sock
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|s| s.try_clone().ok()));
    let Some(stream) = stream_opt else {
        return;
    };

    run_incoming_stream_loop(env, stream, save_dir, jcallback);
}
