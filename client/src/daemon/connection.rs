// Day 3-4 update: receive path uses FrameDecoder; send path uses encode_ndjson.

use anyhow::{bail, Context, Result};
use tokio::io::AsyncWriteExt;
use tokio::net::unix::OwnedReadHalf;
use tokio::net::UnixStream;
use tokio::io::AsyncReadExt;

use super::discovery::read_discovery;
use super::framing::{encode_ndjson, Frame, FrameDecoder};
use super::wire::{ClientCapabilities, Hello, Inbound};

pub async fn connect() -> Result<()> {
    let disc = read_discovery().context("read daemon discovery")?;

    let stream = UnixStream::connect(&disc.socket_path)
        .await
        .with_context(|| format!("connect to {}", disc.socket_path))?;

    let (mut reader, mut writer) = stream.into_split();

    let caps = ClientCapabilities::stub(220, 50);
    let hello = Hello::new(disc.token.clone(), caps);
    let frame_bytes = encode_ndjson(&hello).map_err(|e| anyhow::anyhow!("{e}"))?;
    writer.write_all(&frame_bytes).await.context("send hello")?;

    let frame = recv_one_frame(&mut reader).await?;
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
        }
        Inbound::Fatal(f) => bail!("daemon rejected connection: {} — {}", f.code, f.message),
        _ => bail!("expected helloAck, got unexpected envelope"),
    }

    Ok(())
}

async fn recv_one_frame(stream: &mut OwnedReadHalf) -> Result<Frame> {
    let mut decoder = FrameDecoder::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream.read(&mut buf).await.context("read frame")?;
        if n == 0 {
            bail!("connection closed before complete frame");
        }
        decoder.push(&buf[..n]);
        let mut frames = decoder.drain().map_err(|e| anyhow::anyhow!("{e}"))?;
        if !frames.is_empty() {
            return Ok(frames.remove(0));
        }
    }
}
