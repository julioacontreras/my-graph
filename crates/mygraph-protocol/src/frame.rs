//! Frame layout (little-endian):
//!
//! ```text
//! u8  version         PROTOCOL_VERSION
//! u8  message type    see MessageType
//! u32 request id      chosen by the client, echoed in the response
//! u32 payload length  at most MAX_PAYLOAD
//! ... payload
//! ```

use std::io::{self, Read, Write};

pub const PROTOCOL_VERSION: u8 = 1;
pub const HEADER_LEN: usize = 10;
/// Largest payload accepted in either direction (64 MiB).
pub const MAX_PAYLOAD: u32 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub version: u8,
    pub message_type: u8,
    pub request_id: u32,
    pub payload: Vec<u8>,
}

/// Reads one frame. Returns `Ok(None)` on a clean end of stream before the
/// first header byte.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Frame>> {
    let mut header = [0u8; HEADER_LEN];
    let mut filled = 0;
    while filled < HEADER_LEN {
        match r.read(&mut header[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    let len = u32::from_le_bytes(header[6..10].try_into().unwrap());
    if len > MAX_PAYLOAD {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("payload of {len} bytes exceeds the limit")));
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload)?;
    Ok(Some(Frame {
        version: header[0],
        message_type: header[1],
        request_id: u32::from_le_bytes(header[2..6].try_into().unwrap()),
        payload,
    }))
}

/// Writes one frame with the current protocol version and flushes.
pub fn write_frame(w: &mut impl Write, message_type: u8, request_id: u32, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len())
        .ok()
        .filter(|l| *l <= MAX_PAYLOAD)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;
    let mut header = [0u8; HEADER_LEN];
    header[0] = PROTOCOL_VERSION;
    header[1] = message_type;
    header[2..6].copy_from_slice(&request_id.to_le_bytes());
    header[6..10].copy_from_slice(&len.to_le_bytes());
    w.write_all(&header)?;
    w.write_all(payload)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_eof() {
        let mut buf = Vec::new();
        write_frame(&mut buf, 0x10, 99, b"abc").unwrap();
        let mut r = &buf[..];
        let f = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(f, Frame { version: 1, message_type: 0x10, request_id: 99, payload: b"abc".to_vec() });
        assert!(read_frame(&mut r).unwrap().is_none());
    }

    #[test]
    fn truncated_and_oversized() {
        let mut buf = Vec::new();
        write_frame(&mut buf, 1, 1, b"abc").unwrap();
        assert!(read_frame(&mut &buf[..4]).is_err());
        assert!(read_frame(&mut &buf[..buf.len() - 1]).is_err());
        let mut big = vec![1, 1, 0, 0, 0, 0];
        big.extend_from_slice(&(MAX_PAYLOAD + 1).to_le_bytes());
        assert!(read_frame(&mut &big[..]).is_err());
    }
}
