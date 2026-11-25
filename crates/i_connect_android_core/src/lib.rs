//! Pure Rust Android JNI engine replacing the legacy C socket core.
//! Handles UDP broadcast & mobile-hotspot auto-discovery, TCP_NODELAY control
//! & auth handshake, heartbeat, input/media/system packet framing, and turbo
//! 4MB-buffered bidirectional batch file/folder transfer with throttled UI callbacks.

use i_connect_proto::{
    sanitize_relative_path, send_frame, CHUNK_BUFFER_SIZE, DISCOVER_PING_MAGIC,
    DISCOVER_PONG_MAGIC, PROTO_MAGIC, REMOTE_UDP_PORT, SOCKET_BUFFER_SIZE,
};
use i_connect_proto::{
    MsgAuthReq, MsgAuthResp, MsgFileBatchHeader, MsgFileItemHeader, MsgKeyEvent, MsgMediaCmd,
    MsgMouseClick, MsgMouseMove, MsgMouseScroll, MsgSysCmd, MsgTextInput, MsgType, RemoteHeader,
};
use jni::objects::{JClass, JObject, JObjectArray, JString, JValue};
use jni::sys::{jboolean, jint, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use log::{error, info};
use socket2::{Domain, Protocol, Socket, Type};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

struct CoreState {
    control_sock: Mutex<Option<TcpStream>>,
    file_sock: Mutex<Option<TcpStream>>,
    device_id: Mutex<String>,
    device_name: Mutex<String>,
}

static CORE: OnceLock<CoreState> = OnceLock::new();

fn core() -> &'static CoreState {
    CORE.get_or_init(|| CoreState {
        control_sock: Mutex::new(None),
        file_sock: Mutex::new(None),
        device_id: Mutex::new("android-device-default".to_string()),
        device_name: Mutex::new("Android Phone".to_string()),
    })
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

fn configure_turbo_socket(stream: &TcpStream) {
    let sock_ref = socket2::SockRef::from(stream);
    let _ = sock_ref.set_tcp_nodelay(true);
    let _ = sock_ref.set_recv_buffer_size(SOCKET_BUFFER_SIZE as usize);
    let _ = sock_ref.set_send_buffer_size(SOCKET_BUFFER_SIZE as usize);
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
    info!("I Connect Rust NativeCore initialized");
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
        // 1. General local broadcast
        let _ = udp.send_to(ping, SocketAddrV4::new(Ipv4Addr::BROADCAST, REMOTE_UDP_PORT));
        // 2. Android Mobile Hotspot directed broadcast (192.168.43.255)
        let _ = udp.send_to(
            ping,
            SocketAddrV4::new(Ipv4Addr::new(192, 168, 43, 255), REMOTE_UDP_PORT),
        );
        // 3. Common Hotspot client unicast pings (192.168.43.2 .. 40)
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
        return 0; // Denied by PC
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

    let addr = SocketAddr::V4(SocketAddrV4::new(ip, port as u16));
    let stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
        Ok(s) => s,
        Err(_) => return -2,
    };
    configure_turbo_socket(&stream);

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
    configure_turbo_socket(&stream);

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
        let mut reader = BufReader::with_capacity(2 * 1024 * 1024, f);

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
    let mut stream = match stream_opt {
        Some(s) => s,
        None => return,
    };
    configure_turbo_socket(&stream);

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
                    .map(|f| BufWriter::with_capacity(2 * 1024 * 1024, f));

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
