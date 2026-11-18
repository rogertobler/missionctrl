//! herdr-core's framed mode: several channels on one connection, needed for
//! `terminal.attach` (herdr-core `schema/README.md`, "Framed mode" and
//! "Attach streams", protocol generation 1).
//!
//! A connection starts in plain mode; `hello {framed: true}` switches it after
//! the reply. Every frame is `[channel u32 LE][kind u8][len u32 LE][payload]`.
//! Channel 0 carries JSON-RPC messages; an attach answers with a channel of its
//! own that carries the terminal's bytes.

use serde_json::{Value, json};
use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, ReadHalf, WriteHalf,
};

use super::client::{Endpoint, GENERATION, open_endpoint, parse_message};
use crate::error::{Error, Result};
use crate::herdr::read_line;
use crate::transport::{BoxStream, Transport};

/// One message (channel 0) or stream bytes (other channels).
pub const DATA: u8 = 0;
/// u32 LE: bytes the receiver consumed.
pub const CREDIT: u8 = 1;
/// The channel is over; from the core with a JSON reason.
pub const CLOSE: u8 = 2;
/// u16 LE cols, u16 LE rows: viewer → core, and core → observers that asked.
pub const RESIZE: u8 = 3;

/// Viewer → core: 1 gained focus, 0 lost it.
pub const FOCUS: u8 = 4;

/// Largest payload the protocol allows.
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub channel: u32,
    pub kind: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn data(channel: u32, payload: Vec<u8>) -> Self {
        Self {
            channel,
            kind: DATA,
            payload,
        }
    }

    pub fn credit(channel: u32, bytes: u32) -> Self {
        Self {
            channel,
            kind: CREDIT,
            payload: bytes.to_le_bytes().to_vec(),
        }
    }

    pub fn close(channel: u32) -> Self {
        Self {
            channel,
            kind: CLOSE,
            payload: Vec::new(),
        }
    }

    pub fn focus(channel: u32, gained: bool) -> Self {
        Self {
            channel,
            kind: FOCUS,
            payload: vec![u8::from(gained)],
        }
    }

    pub fn resize(channel: u32, cols: u16, rows: u16) -> Self {
        let mut payload = cols.to_le_bytes().to_vec();
        payload.extend_from_slice(&rows.to_le_bytes());
        Self {
            channel,
            kind: RESIZE,
            payload,
        }
    }

    /// `(cols, rows)` of a RESIZE frame.
    pub fn size(&self) -> Option<(u16, u16)> {
        (self.kind == RESIZE && self.payload.len() >= 4).then(|| {
            (
                u16::from_le_bytes([self.payload[0], self.payload[1]]),
                u16::from_le_bytes([self.payload[2], self.payload[3]]),
            )
        })
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(9 + self.payload.len());
        out.extend_from_slice(&self.channel.to_le_bytes());
        out.push(self.kind);
        out.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.payload);
        out
    }
}

/// The next frame; `None` when the stream ended between frames.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<Frame>> {
    let mut head = [0u8; 9];
    match reader.read_exact(&mut head[..1]).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    reader.read_exact(&mut head[1..]).await?;
    let channel = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let kind = head[4];
    let len = u32::from_le_bytes([head[5], head[6], head[7], head[8]]) as usize;
    if len > MAX_PAYLOAD {
        return Err(Error::Protocol(format!("frame of {len} bytes")));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    Ok(Some(Frame {
        channel,
        kind,
        payload,
    }))
}

pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &Frame) -> Result<()> {
    writer.write_all(&frame.encode()).await?;
    writer.flush().await?;
    Ok(())
}

pub type FramedReader = ReadHalf<BufReader<BoxStream>>;
pub type FramedWriter = WriteHalf<BufReader<BoxStream>>;

/// A connection switched to framed mode, with the `hello` reply.
pub struct Framed {
    pub reader: FramedReader,
    pub writer: FramedWriter,
    pub features: Vec<String>,
    next_id: u64,
}

impl Framed {
    /// Open `endpoint` and switch it to framed mode.
    pub async fn open(transport: &Transport, endpoint: &Endpoint) -> Result<Self> {
        let mut stream = BufReader::new(open_endpoint(transport, endpoint).await?);
        let mut line = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 0, "method": "hello",
            "params": { "framed": true, "generation": GENERATION },
        }))?;
        line.push(b'\n');
        stream.get_mut().write_all(&line).await?;
        stream.get_mut().flush().await?;
        let reply = read_line(&mut stream, &mut Vec::new())
            .await?
            .ok_or_else(|| Error::Protocol("hello: connection closed without response".into()))?;
        let hello = parse_message(&reply, 0)?
            .ok_or_else(|| Error::Protocol("hello: no response".into()))?;
        if hello.get("framed").and_then(Value::as_bool) != Some(true) {
            return Err(Error::Protocol("core did not switch to framed mode".into()));
        }
        let features = hello
            .get("features")
            .and_then(Value::as_array)
            .map(|f| {
                f.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        // The BufReader keeps bytes that came after the reply line; splitting it
        // (not the inner stream) keeps them for the first frame.
        let (reader, writer) = tokio::io::split(stream);
        Ok(Self {
            reader,
            writer,
            features,
            next_id: 1,
        })
    }

    /// Send a JSON-RPC request on channel 0 and wait for its answer. Frames on
    /// other channels meanwhile are skipped (the core answers an attach before
    /// it sends that attach's stream frames; earlier attaches may still send).
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let message = serde_json::to_vec(
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )?;
        write_frame(&mut self.writer, &Frame::data(0, message)).await?;
        loop {
            let frame = read_frame(&mut self.reader)
                .await?
                .ok_or_else(|| Error::Protocol(format!("{method}: connection closed")))?;
            if frame.channel != 0 || frame.kind != DATA {
                continue;
            }
            let text = String::from_utf8_lossy(&frame.payload);
            if let Some(result) = parse_message(&text, id)? {
                return Ok(result);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_round_trip() {
        let frames = [
            Frame::data(0, b"{}".to_vec()),
            Frame::credit(3, 4096),
            Frame::resize(3, 80, 24),
            Frame::close(3),
        ];
        let mut bytes = Vec::new();
        for f in &frames {
            write_frame(&mut bytes, f).await.unwrap();
        }
        // Layout of the first frame: channel, kind, length, payload.
        assert_eq!(&bytes[..11], &[0, 0, 0, 0, 0, 2, 0, 0, 0, b'{', b'}']);
        let mut reader = bytes.as_slice();
        for f in &frames {
            assert_eq!(read_frame(&mut reader).await.unwrap().as_ref(), Some(f));
        }
        assert_eq!(read_frame(&mut reader).await.unwrap(), None);
        assert_eq!(frames[2].size(), Some((80, 24)));
    }

    #[tokio::test]
    async fn oversized_frame_is_refused() {
        let mut bytes = vec![1, 0, 0, 0, DATA];
        bytes.extend_from_slice(&((MAX_PAYLOAD as u32) + 1).to_le_bytes());
        assert!(read_frame(&mut bytes.as_slice()).await.is_err());
    }
}
