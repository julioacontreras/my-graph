//! MyGraph wire protocol, version 1. See `docs/PROTOCOL.md`.
//!
//! A connection carries frames. Each frame has a fixed 10-byte header
//! (version, message type, request id, payload length) followed by a payload.
//! The payload is always one encoded MAP value. This crate does no I/O beyond
//! `Read`/`Write` and has no knowledge of sockets or the query engine.

pub mod codec;
pub mod frame;
pub mod message;

pub use frame::{read_frame, write_frame, Frame, HEADER_LEN, MAX_PAYLOAD, PROTOCOL_VERSION};
pub use message::{error_payload, MessageType, Payload};
