// Day 2 — UDS connect + hello/helloAck handshake + bootId pinning.
//
// Framing (arch §10, framing.ts):
//   0x00 NDJSON : [0x00][json bytes][0x0a]         (256 KB max)
//   0x01 PTY out: [0x01][streamId:u32be][len:u32be][bytes]  (1 MB max)
//   0x02 asset  : [0x02][assetId:u32be][tier:u8][len:u32be][bytes]
//   0x03 PTY in : [0x03][streamId:u32be][len:u32be][bytes]  (1 MB max)

use anyhow::{bail, Context, Result};
use bytes::{BufMut, BytesMut};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use super::discovery::read_discovery;
use super::wire::{ClientCapabilities, Hello, Inbound};

const NDJSON_TAG: u8 = 0x00;
const NDJSON_CAP: usize = 256 * 1024;

pub async fn connect() -> Result<()> {
    let disc = read_discovery().context("read daemon discovery")?;

    let stream = UnixStream::connect(&disc.socket_path)
        .await
        .with_context(|| format!("connect to {}", disc.socket_path))?;

    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    // Send hello — [0x00][json]\n
    let caps = ClientCapabilities::stub(220, 50);
    let hello = Hello::new(disc.token.clone(), caps);
    send_ndjson(&mut writer, &hello).await?;

    // Await helloAck — read one NDJSON frame
    let line = recv_ndjson_line(&mut reader).await?;
    let ack: Inbound = serde_json::from_str(&line).context("parse helloAck")?;

    match ack {
        Inbound::HelloAck(ack) => {
            if ack.boot_id != disc.boot_id {
                bail!(
                    "bootId mismatch: discovery={} ack={}",
                    disc.boot_id,
                    ack.boot_id
                );
            }
            eprintln!(
                "connected: daemon {} boot={} session={}",
                ack.daemon_version,
                &ack.boot_id[..8],
                &ack.session_id[..8]
            );
        }
        Inbound::Fatal(f) => bail!("daemon rejected connection: {} — {}", f.code, f.message),
        _ => bail!("expected helloAck, got unexpected envelope"),
    }

    Ok(())
}

// ── Framing helpers ───────────────────────────────────────────────────────────

async fn send_ndjson<W: AsyncWriteExt + Unpin, T: serde::Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<()> {
    let payload = serde_json::to_vec(value)?;
    if payload.len() > NDJSON_CAP {
        bail!("outbound NDJSON frame too large: {} bytes", payload.len());
    }
    let mut buf = BytesMut::with_capacity(2 + payload.len());
    buf.put_u8(NDJSON_TAG);
    buf.extend_from_slice(&payload);
    buf.put_u8(b'\n');
    writer.write_all(&buf).await.context("write NDJSON frame")?;
    Ok(())
}

/// Read one NDJSON frame: skip the 0x00 tag byte, read until `\n`.
async fn recv_ndjson_line<R: AsyncBufReadExt + Unpin>(reader: &mut R) -> Result<String> {
    // Peek / consume the tag byte.
    let mut tag = [0u8; 1];
    reader.read_exact(&mut tag).await.context("read frame tag")?;
    if tag[0] != NDJSON_TAG {
        bail!("expected NDJSON tag 0x00, got 0x{:02x}", tag[0]);
    }

    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .await
        .context("read NDJSON line")?;
    if n == 0 {
        bail!("connection closed before helloAck");
    }
    // Strip trailing newline.
    if line.ends_with('\n') {
        line.pop();
    }
    Ok(line)
}
