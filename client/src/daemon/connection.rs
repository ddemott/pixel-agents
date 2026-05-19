use anyhow::{bail, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;

use super::discovery::read_discovery;
use super::framing::{encode_ndjson, encode_pty_in, Frame, FrameDecoder};
use super::wire::{ClientCapabilities, Hello, Inbound};

/// Live authenticated connection to the daemon.
#[allow(dead_code)]
pub struct DaemonConn {
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
    decoder: FrameDecoder,
}

#[allow(dead_code)]
impl DaemonConn {
    /// Receive the next frame from the daemon. Decoder state preserved across
    /// partial reads so this is safe to cancel and re-poll.
    pub async fn recv_frame(&mut self) -> Result<Frame> {
        let mut buf = [0u8; 8192];
        loop {
            {
                let mut frames = self.decoder.drain().map_err(|e| anyhow::anyhow!("{e}"))?;
                if !frames.is_empty() {
                    return Ok(frames.remove(0));
                }
            }
            let n = self.reader.read(&mut buf).await.context("read from daemon")?;
            if n == 0 {
                bail!("daemon closed connection");
            }
            self.decoder.push(&buf[..n]);
        }
    }

    /// Send a JSON-encodable message as an NDJSON frame.
    pub async fn send<T: serde::Serialize>(&mut self, msg: &T) -> Result<()> {
        let bytes = encode_ndjson(msg).map_err(|e| anyhow::anyhow!("{e}"))?;
        self.writer.write_all(&bytes).await.context("send to daemon")
    }

    /// Send PTY input bytes for the given stream_id.
    pub async fn send_pty_in(&mut self, stream_id: u32, data: &[u8]) -> Result<()> {
        let bytes = encode_pty_in(stream_id, data).map_err(|e| anyhow::anyhow!("{e}"))?;
        self.writer.write_all(&bytes).await.context("send pty_in")
    }
}

/// Connect to the daemon, complete the hello/helloAck handshake, and return
/// the authenticated `DaemonConn` ready for the event loop.
pub async fn connect(caps: ClientCapabilities) -> Result<DaemonConn> {
    let disc = read_discovery().context("read daemon discovery")?;

    let stream = UnixStream::connect(&disc.socket_path)
        .await
        .with_context(|| format!("connect to {}", disc.socket_path))?;

    let (mut reader, mut writer) = stream.into_split();
    let mut decoder = FrameDecoder::new();

    let hello = Hello::new(disc.token.clone(), caps);
    let frame_bytes = encode_ndjson(&hello).map_err(|e| anyhow::anyhow!("{e}"))?;
    writer.write_all(&frame_bytes).await.context("send hello")?;

    let frame = recv_frame_raw(&mut reader, &mut decoder).await?;
    let Frame::Ndjson(json) = frame else {
        bail!("expected NDJSON frame for helloAck, got binary frame");
    };
    let ack: Inbound = serde_json::from_str(&json).context("parse helloAck")?;

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
            Ok(DaemonConn { reader, writer, decoder })
        }
        Inbound::Fatal(f) => bail!("daemon rejected connection: {} — {}", f.code, f.message),
        _ => bail!("expected helloAck, got unexpected envelope"),
    }
}

async fn recv_frame_raw(stream: &mut OwnedReadHalf, decoder: &mut FrameDecoder) -> Result<Frame> {
    let mut buf = [0u8; 8192];
    loop {
        {
            let mut frames = decoder.drain().map_err(|e| anyhow::anyhow!("{e}"))?;
            if !frames.is_empty() {
                return Ok(frames.remove(0));
            }
        }
        let n = stream.read(&mut buf).await.context("read frame")?;
        if n == 0 {
            bail!("connection closed before complete frame");
        }
        decoder.push(&buf[..n]);
    }
}
