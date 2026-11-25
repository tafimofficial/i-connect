//! Binary network protocol shared between the I Connect Windows PC Server
//! and the I Connect Android native core.
//!
//! Wire layout matches the exact little-endian packed framing (`"RCP1"` magic,
//! 8-byte header, fixed-size payloads) on UDP 52199, Control TCP 52200, and
//! Turbo File TCP 52201.

use std::io::{self, Read, Write};

pub const REMOTE_UDP_PORT: u16 = 52199;
pub const REMOTE_CONTROL_PORT: u16 = 52200;
pub const REMOTE_FILE_PORT: u16 = 52201;

pub const PROTO_MAGIC: u32 = 0x5243_5031; // "RCP1"
pub const DISCOVER_PING_MAGIC: &str = "RCP_DISCOVER_SERVER";
pub const DISCOVER_PONG_MAGIC: &str = "RCP_SERVER_ANNOUNCE";

pub const CHUNK_BUFFER_SIZE: usize = 1024 * 1024;
pub const SOCKET_BUFFER_SIZE: i32 = 8 * 1024 * 1024;
pub const PARALLEL_FILE_STREAMS: usize = 4;
pub const STRIPE_SIZE: u64 = 4 * 1024 * 1024;

pub const HEADER_SIZE: usize = 8;

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    AuthReq = 1,
    AuthResp = 2,
    Heartbeat = 3,

    MouseMove = 10,
    MouseClick = 11,
    MouseScroll = 12,

    KeyEvent = 20,
    TextInput = 21,

    MediaCmd = 30,
    SysCmd = 40,
    StatusUpdate = 50,

    FileBatchHeader = 60,
    FileItemHeader = 61,
    FileDataChunk = 62,
    FileItemFinish = 63,
    FileBatchFinish = 64,
    FileRangeHeader = 65,
    FileChannelHello = 66,
}

impl MsgType {
    pub fn from_u16(val: u16) -> Option<Self> {
        match val {
            1 => Some(Self::AuthReq),
            2 => Some(Self::AuthResp),
            3 => Some(Self::Heartbeat),
            10 => Some(Self::MouseMove),
            11 => Some(Self::MouseClick),
            12 => Some(Self::MouseScroll),
            20 => Some(Self::KeyEvent),
            21 => Some(Self::TextInput),
            30 => Some(Self::MediaCmd),
            40 => Some(Self::SysCmd),
            50 => Some(Self::StatusUpdate),
            60 => Some(Self::FileBatchHeader),
            61 => Some(Self::FileItemHeader),
            62 => Some(Self::FileDataChunk),
            63 => Some(Self::FileItemFinish),
            64 => Some(Self::FileBatchFinish),
            65 => Some(Self::FileRangeHeader),
            66 => Some(Self::FileChannelHello),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left = 1,
    Right = 2,
    Middle = 3,
}

impl MouseButton {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Left),
            2 => Some(Self::Right),
            3 => Some(Self::Middle),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Down = 0,
    Up = 1,
    Click = 2,
    DoubleClick = 3,
}

impl MouseAction {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Down),
            1 => Some(Self::Up),
            2 => Some(Self::Click),
            3 => Some(Self::DoubleClick),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Down = 0,
    Up = 1,
    Press = 2,
}

impl KeyAction {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Down),
            1 => Some(Self::Up),
            2 => Some(Self::Press),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    PlayPause = 1,
    Prev = 2,
    Next = 3,
    VolUp = 4,
    VolDown = 5,
    Mute = 6,
    SeekBack = 7,
    SeekFwd = 8,
    SeekPos = 9,
}

impl MediaAction {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::PlayPause),
            2 => Some(Self::Prev),
            3 => Some(Self::Next),
            4 => Some(Self::VolUp),
            5 => Some(Self::VolDown),
            6 => Some(Self::Mute),
            7 => Some(Self::SeekBack),
            8 => Some(Self::SeekFwd),
            9 => Some(Self::SeekPos),
            _ => None,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysCommand {
    Lock = 1,
    Sleep = 2,
    Shutdown = 3,
    Restart = 4,
    ShowDesktop = 5,
    CloseWindow = 6,
    TaskManager = 7,
}

impl SysCommand {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Lock),
            2 => Some(Self::Sleep),
            3 => Some(Self::Shutdown),
            4 => Some(Self::Restart),
            5 => Some(Self::ShowDesktop),
            6 => Some(Self::CloseWindow),
            7 => Some(Self::TaskManager),
            _ => None,
        }
    }
}

/// Standard 8-byte wire frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteHeader {
    pub magic: u32,
    pub msg_type: u16,
    pub payload_len: u16,
}

impl RemoteHeader {
    pub fn new(msg_type: MsgType, payload_len: u16) -> Self {
        Self {
            magic: PROTO_MAGIC,
            msg_type: msg_type as u16,
            payload_len,
        }
    }

    pub fn to_bytes(self) -> [u8; HEADER_SIZE] {
        let mut out = [0u8; HEADER_SIZE];
        out[0..4].copy_from_slice(&self.magic.to_le_bytes());
        out[4..6].copy_from_slice(&self.msg_type.to_le_bytes());
        out[6..8].copy_from_slice(&self.payload_len.to_le_bytes());
        out
    }

    pub fn from_bytes(buf: &[u8; HEADER_SIZE]) -> Self {
        Self {
            magic: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            msg_type: u16::from_le_bytes([buf[4], buf[5]]),
            payload_len: u16::from_le_bytes([buf[6], buf[7]]),
        }
    }

    pub fn read_from<R: Read>(reader: &mut R) -> io::Result<Self> {
        let mut buf = [0u8; HEADER_SIZE];
        reader.read_exact(&mut buf)?;
        Ok(Self::from_bytes(&buf))
    }

    pub fn write_to<W: Write>(self, writer: &mut W) -> io::Result<()> {
        writer.write_all(&self.to_bytes())
    }
}

pub fn write_fixed_cstr(dest: &mut [u8], s: &str) {
    dest.fill(0);
    if dest.is_empty() {
        return;
    }
    let max_copy = dest.len() - 1;
    let bytes = s.as_bytes();
    let mut len = bytes.len().min(max_copy);
    // Avoid splitting a multi-byte UTF-8 character at the boundary
    while len > 0 && !s.is_char_boundary(len) {
        len -= 1;
    }
    dest[..len].copy_from_slice(&bytes[..len]);
}

pub fn read_fixed_cstr(src: &[u8]) -> String {
    let end = src.iter().position(|&b| b == 0).unwrap_or(src.len());
    String::from_utf8_lossy(&src[..end]).into_owned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgAuthReq {
    pub device_id: String,
    pub device_name: String,
}

impl MsgAuthReq {
    pub const WIRE_LEN: usize = 128;

    pub fn encode(&self) -> [u8; Self::WIRE_LEN] {
        let mut out = [0u8; Self::WIRE_LEN];
        write_fixed_cstr(&mut out[0..64], &self.device_id);
        write_fixed_cstr(&mut out[64..128], &self.device_name);
        out
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "AuthReq too short",
            ));
        }
        Ok(Self {
            device_id: read_fixed_cstr(&buf[0..64]),
            device_name: read_fixed_cstr(&buf[64..128]),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgAuthResp {
    pub status: u8, // 1 = Accepted, 0 = Denied, 2 = Pending
    pub message: String,
}

impl MsgAuthResp {
    pub const WIRE_LEN: usize = 65;

    pub fn encode(&self) -> [u8; Self::WIRE_LEN] {
        let mut out = [0u8; Self::WIRE_LEN];
        out[0] = self.status;
        write_fixed_cstr(&mut out[1..65], &self.message);
        out
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "AuthResp too short",
            ));
        }
        Ok(Self {
            status: buf[0],
            message: read_fixed_cstr(&buf[1..65]),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgMouseMove {
    pub dx: i16,
    pub dy: i16,
    pub smooth: u8,
}

impl MsgMouseMove {
    pub const WIRE_LEN: usize = 5;

    pub fn encode(self) -> [u8; Self::WIRE_LEN] {
        let mut out = [0u8; Self::WIRE_LEN];
        out[0..2].copy_from_slice(&self.dx.to_le_bytes());
        out[2..4].copy_from_slice(&self.dy.to_le_bytes());
        out[4] = self.smooth;
        out
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "MouseMove too short",
            ));
        }
        Ok(Self {
            dx: i16::from_le_bytes([buf[0], buf[1]]),
            dy: i16::from_le_bytes([buf[2], buf[3]]),
            smooth: buf[4],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgMouseClick {
    pub button: u8,
    pub action: u8,
}

impl MsgMouseClick {
    pub const WIRE_LEN: usize = 2;

    pub fn encode(self) -> [u8; Self::WIRE_LEN] {
        [self.button, self.action]
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "MouseClick too short",
            ));
        }
        Ok(Self {
            button: buf[0],
            action: buf[1],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgMouseScroll {
    pub delta_y: i16,
    pub delta_x: i16,
}

impl MsgMouseScroll {
    pub const WIRE_LEN: usize = 4;

    pub fn encode(self) -> [u8; Self::WIRE_LEN] {
        let mut out = [0u8; Self::WIRE_LEN];
        out[0..2].copy_from_slice(&self.delta_y.to_le_bytes());
        out[2..4].copy_from_slice(&self.delta_x.to_le_bytes());
        out
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "MouseScroll too short",
            ));
        }
        Ok(Self {
            delta_y: i16::from_le_bytes([buf[0], buf[1]]),
            delta_x: i16::from_le_bytes([buf[2], buf[3]]),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgKeyEvent {
    pub vk_code: u16,
    pub action: u8,
    pub modifiers: u8,
}

impl MsgKeyEvent {
    pub const WIRE_LEN: usize = 4;

    pub fn encode(self) -> [u8; Self::WIRE_LEN] {
        let mut out = [0u8; Self::WIRE_LEN];
        out[0..2].copy_from_slice(&self.vk_code.to_le_bytes());
        out[2] = self.action;
        out[3] = self.modifiers;
        out
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "KeyEvent too short",
            ));
        }
        Ok(Self {
            vk_code: u16::from_le_bytes([buf[0], buf[1]]),
            action: buf[2],
            modifiers: buf[3],
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgTextInput {
    pub char_count: u16,
    pub text: String,
}

impl MsgTextInput {
    pub const WIRE_LEN: usize = 258;

    pub fn new(text: &str) -> Self {
        Self {
            char_count: text.len().min(255) as u16,
            text: text.to_string(),
        }
    }

    pub fn encode(&self) -> [u8; Self::WIRE_LEN] {
        let mut out = [0u8; Self::WIRE_LEN];
        out[0..2].copy_from_slice(&self.char_count.to_le_bytes());
        write_fixed_cstr(&mut out[2..258], &self.text);
        out
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "TextInput too short",
            ));
        }
        Ok(Self {
            char_count: u16::from_le_bytes([buf[0], buf[1]]),
            text: read_fixed_cstr(&buf[2..258]),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgMediaCmd {
    pub action: u8,
    pub value: u8,
}

impl MsgMediaCmd {
    pub const WIRE_LEN: usize = 2;

    pub fn encode(self) -> [u8; Self::WIRE_LEN] {
        [self.action, self.value]
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "MediaCmd too short",
            ));
        }
        Ok(Self {
            action: buf[0],
            value: buf[1],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgSysCmd {
    pub cmd: u8,
}

impl MsgSysCmd {
    pub const WIRE_LEN: usize = 1;

    pub fn encode(self) -> [u8; Self::WIRE_LEN] {
        [self.cmd]
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "SysCmd too short",
            ));
        }
        Ok(Self { cmd: buf[0] })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgFileBatchHeader {
    pub total_files: u32,
    pub total_bytes: u64,
}

impl MsgFileBatchHeader {
    pub const WIRE_LEN: usize = 12;

    pub fn encode(self) -> [u8; Self::WIRE_LEN] {
        let mut out = [0u8; Self::WIRE_LEN];
        out[0..4].copy_from_slice(&self.total_files.to_le_bytes());
        out[4..12].copy_from_slice(&self.total_bytes.to_le_bytes());
        out
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "FileBatchHeader too short",
            ));
        }
        Ok(Self {
            total_files: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            total_bytes: u64::from_le_bytes([
                buf[4], buf[5], buf[6], buf[7], buf[8], buf[9], buf[10], buf[11],
            ]),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgFileItemHeader {
    pub file_index: u32,
    pub file_size: u64,
    pub name_length: u16,
    pub file_name: String,
}

impl MsgFileItemHeader {
    pub const WIRE_LEN: usize = 270;

    pub fn new(file_index: u32, file_size: u64, file_name: &str) -> Self {
        let safe_name = file_name.replace('\\', "/");
        let name_len = safe_name.len().min(255) as u16;
        Self {
            file_index,
            file_size,
            name_length: name_len,
            file_name: safe_name,
        }
    }

    pub fn encode(&self) -> [u8; Self::WIRE_LEN] {
        let mut out = [0u8; Self::WIRE_LEN];
        out[0..4].copy_from_slice(&self.file_index.to_le_bytes());
        out[4..12].copy_from_slice(&self.file_size.to_le_bytes());
        out[12..14].copy_from_slice(&self.name_length.to_le_bytes());
        write_fixed_cstr(&mut out[14..270], &self.file_name);
        out
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "FileItemHeader too short",
            ));
        }
        Ok(Self {
            file_index: u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            file_size: u64::from_le_bytes([
                buf[4], buf[5], buf[6], buf[7], buf[8], buf[9], buf[10], buf[11],
            ]),
            name_length: u16::from_le_bytes([buf[12], buf[13]]),
            file_name: read_fixed_cstr(&buf[14..270]),
        })
    }
}

/// Multi-stream parallel byte-range stripe header (304 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgFileRangeHeader {
    pub batch_id: u64,
    pub file_index: u32,
    pub total_files: u32,
    pub batch_total_bytes: u64,
    pub file_size: u64,
    pub offset: u64,
    pub chunk_len: u64,
    pub file_name: String,
}

impl MsgFileRangeHeader {
    pub const WIRE_LEN: usize = 304;

    pub fn new(
        batch_id: u64,
        file_index: u32,
        total_files: u32,
        batch_total_bytes: u64,
        file_size: u64,
        offset: u64,
        chunk_len: u64,
        file_name: &str,
    ) -> Self {
        Self {
            batch_id,
            file_index,
            total_files,
            batch_total_bytes,
            file_size,
            offset,
            chunk_len,
            file_name: file_name.replace('\\', "/"),
        }
    }

    pub fn encode(&self) -> [u8; Self::WIRE_LEN] {
        let mut out = [0u8; Self::WIRE_LEN];
        out[0..8].copy_from_slice(&self.batch_id.to_le_bytes());
        out[8..12].copy_from_slice(&self.file_index.to_le_bytes());
        out[12..16].copy_from_slice(&self.total_files.to_le_bytes());
        out[16..24].copy_from_slice(&self.batch_total_bytes.to_le_bytes());
        out[24..32].copy_from_slice(&self.file_size.to_le_bytes());
        out[32..40].copy_from_slice(&self.offset.to_le_bytes());
        out[40..48].copy_from_slice(&self.chunk_len.to_le_bytes());
        write_fixed_cstr(&mut out[48..304], &self.file_name);
        out
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "FileRangeHeader too short",
            ));
        }
        let u64_at = |o: usize| {
            u64::from_le_bytes([
                buf[o],
                buf[o + 1],
                buf[o + 2],
                buf[o + 3],
                buf[o + 4],
                buf[o + 5],
                buf[o + 6],
                buf[o + 7],
            ])
        };
        Ok(Self {
            batch_id: u64_at(0),
            file_index: u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]),
            total_files: u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]),
            batch_total_bytes: u64_at(16),
            file_size: u64_at(24),
            offset: u64_at(32),
            chunk_len: u64_at(40),
            file_name: read_fixed_cstr(&buf[48..304]),
        })
    }
}

/// Identifies a pre-connected pooled worker channel from Phone -> PC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgFileChannelHello {
    pub role: u8, // 1 = incoming receiver pool worker
    pub device_id: String,
}

impl MsgFileChannelHello {
    pub const WIRE_LEN: usize = 65;

    pub fn encode(&self) -> [u8; Self::WIRE_LEN] {
        let mut out = [0u8; Self::WIRE_LEN];
        out[0] = self.role;
        write_fixed_cstr(&mut out[1..65], &self.device_id);
        out
    }

    pub fn decode(buf: &[u8]) -> io::Result<Self> {
        if buf.len() < Self::WIRE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "FileChannelHello too short",
            ));
        }
        Ok(Self {
            role: buf[0],
            device_id: read_fixed_cstr(&buf[1..65]),
        })
    }
}

/// Write a framed packet (`RemoteHeader` + payload) in a single contiguous syscall.
pub fn send_frame<W: Write>(writer: &mut W, msg_type: MsgType, payload: &[u8]) -> io::Result<()> {
    let hdr = RemoteHeader::new(msg_type, payload.len() as u16).to_bytes();
    if payload.is_empty() {
        writer.write_all(&hdr)
    } else {
        let mut frame = Vec::with_capacity(HEADER_SIZE + payload.len());
        frame.extend_from_slice(&hdr);
        frame.extend_from_slice(payload);
        writer.write_all(&frame)
    }
}

/// Sanitize a relative file path received over the network so nested folder trees
/// are preserved while rejecting path traversal (`..`, drive letters, root slashes).
pub fn sanitize_relative_path(raw: &str) -> Vec<String> {
    let normalized = raw.replace('\\', "/");
    let mut parts = Vec::new();
    for seg in normalized.split('/') {
        let trimmed = seg.trim();
        if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
            continue;
        }
        // Strip colon to neutralize Windows drive prefixes (e.g., "C:")
        let clean: String = trimmed
            .chars()
            .filter(|&c| c != ':' && c != '\0' && !matches!(c, '<' | '>' | '"' | '|' | '?' | '*'))
            .collect();
        if !clean.is_empty() {
            parts.push(clean);
        }
    }
    if parts.is_empty() {
        parts.push("unnamed_file".to_string());
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip_matches_c_layout() {
        let hdr = RemoteHeader::new(MsgType::FileItemHeader, MsgFileItemHeader::WIRE_LEN as u16);
        let bytes = hdr.to_bytes();
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[0..4], &PROTO_MAGIC.to_le_bytes());
        let decoded = RemoteHeader::from_bytes(&bytes);
        assert_eq!(decoded, hdr);
    }

    #[test]
    fn payloads_roundtrip() {
        let auth = MsgAuthReq {
            device_id: "uuid-123".into(),
            device_name: "Pixel 9 Pro".into(),
        };
        assert_eq!(MsgAuthReq::decode(&auth.encode()).unwrap(), auth);

        let resp = MsgAuthResp {
            status: 1,
            message: "Authenticated".into(),
        };
        assert_eq!(MsgAuthResp::decode(&resp.encode()).unwrap(), resp);

        let mm = MsgMouseMove {
            dx: -14,
            dy: 27,
            smooth: 1,
        };
        assert_eq!(MsgMouseMove::decode(&mm.encode()).unwrap(), mm);

        let mc = MsgMouseClick {
            button: 2,
            action: 3,
        };
        assert_eq!(MsgMouseClick::decode(&mc.encode()).unwrap(), mc);

        let ms = MsgMouseScroll {
            delta_y: -120,
            delta_x: 64,
        };
        assert_eq!(MsgMouseScroll::decode(&ms.encode()).unwrap(), ms);

        let ke = MsgKeyEvent {
            vk_code: 0x41,
            action: 2,
            modifiers: 0b0010,
        };
        assert_eq!(MsgKeyEvent::decode(&ke.encode()).unwrap(), ke);

        let ti = MsgTextInput::new("Hello 🚀");
        assert_eq!(MsgTextInput::decode(&ti.encode()).unwrap(), ti);

        let media = MsgMediaCmd {
            action: MediaAction::SeekPos as u8,
            value: 50,
        };
        assert_eq!(MsgMediaCmd::decode(&media.encode()).unwrap(), media);

        let sys = MsgSysCmd {
            cmd: SysCommand::ShowDesktop as u8,
        };
        assert_eq!(MsgSysCmd::decode(&sys.encode()).unwrap(), sys);

        let batch = MsgFileBatchHeader {
            total_files: 42,
            total_bytes: 987_654_321,
        };
        assert_eq!(MsgFileBatchHeader::decode(&batch.encode()).unwrap(), batch);

        let item = MsgFileItemHeader::new(3, 123456, "Photos\\2026\\sunset.jpg");
        let decoded_item = MsgFileItemHeader::decode(&item.encode()).unwrap();
        assert_eq!(decoded_item.file_name, "Photos/2026/sunset.jpg");
        assert_eq!(decoded_item.file_size, 123456);

        let range = MsgFileRangeHeader::new(99, 1, 5, 50_000_000, 10_000_000, 4_194_304, 4_194_304, "Videos\\clip.mp4");
        let dec_range = MsgFileRangeHeader::decode(&range.encode()).unwrap();
        assert_eq!(dec_range.batch_id, 99);
        assert_eq!(dec_range.offset, 4_194_304);
        assert_eq!(dec_range.file_name, "Videos/clip.mp4");

        let hello = MsgFileChannelHello {
            role: 1,
            device_id: "uuid-99".into(),
        };
        assert_eq!(MsgFileChannelHello::decode(&hello.encode()).unwrap(), hello);
    }

    #[test]
    fn path_sanitization_strips_traversal() {
        assert_eq!(
            sanitize_relative_path("..\\..\\Windows\\System32\\evil.exe"),
            vec!["Windows", "System32", "evil.exe"]
        );
        assert_eq!(
            sanitize_relative_path("Albums/Trip/img01.png"),
            vec!["Albums", "Trip", "img01.png"]
        );
    }
}
