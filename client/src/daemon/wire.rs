#![allow(dead_code)]

// Wire protocol types — mirrors daemon/src/rpc/wire.ts (protoVersion = 1).
// Binary framing: tag byte + payload.
//   0x00 NDJSON : [0x00][json bytes][0x0a]
//   0x01 PTY out: [0x01][streamId:u32be][len:u32be][bytes]
//   0x02 asset  : [0x02][assetId:u32be][tier:u8][len:u32be][bytes]
//   0x03 PTY in : [0x03][streamId:u32be][len:u32be][bytes]

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTO_VERSION: u32 = 1;
pub const CLIENT_VERSION: &str = "0.1.0";

// ── Handshake ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Hello {
    pub kind: &'static str, // "hello"
    pub token: String,
    #[serde(rename = "clientVersion")]
    pub client_version: &'static str,
    #[serde(rename = "protoVersion")]
    pub proto_version: u32,
    pub capabilities: ClientCapabilities,
}

impl Hello {
    pub fn new(token: String, capabilities: ClientCapabilities) -> Self {
        Self {
            kind: "hello",
            token,
            client_version: CLIENT_VERSION,
            proto_version: PROTO_VERSION,
            capabilities,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct HelloAck {
    #[serde(rename = "bootId")]
    pub boot_id: String,
    #[serde(rename = "daemonVersion")]
    pub daemon_version: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub world: Value, // WorldSnapshot — parsed lazily
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCapabilities {
    pub rendering: RenderingCap,
    pub cols: u16,
    pub rows: u16,
    #[serde(rename = "cellPx")]
    pub cell_px: CellPx,
    #[serde(rename = "bracketedPaste")]
    pub bracketed_paste: bool,
    pub mouse: bool,
    #[serde(rename = "sixelCols", skip_serializing_if = "Option::is_none")]
    pub sixel_cols: Option<u16>,
    #[serde(rename = "sixelRows", skip_serializing_if = "Option::is_none")]
    pub sixel_rows: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellPx {
    pub w: u16,
    pub h: u16,
}

impl ClientCapabilities {
    pub fn stub(cols: u16, rows: u16) -> Self {
        Self {
            rendering: RenderingCap::Truecolor,
            cols,
            rows,
            cell_px: CellPx { w: 8, h: 16 },
            bracketed_paste: true,
            mouse: true,
            sixel_cols: None,
            sixel_rows: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RenderingCap {
    KittyK,
    KittyO,
    Iterm2,
    Sixel,
    Truecolor,
    #[serde(rename = "256")]
    C256,
    #[serde(rename = "16")]
    C16,
    Braille,
}

// ── RPC ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct Req {
    pub kind: &'static str, // "req"
    #[serde(rename = "reqId")]
    pub req_id: u32,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct Res {
    #[serde(rename = "reqId")]
    pub req_id: u32,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<WireError>,
}

#[derive(Debug, Deserialize)]
pub struct WireError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct Evt {
    pub topic: String,
    pub seq: u64,
    pub ts: u64,
    pub data: Value,
}

#[derive(Debug, Deserialize)]
pub struct Fatal {
    pub code: String,
    pub message: String,
}

// ── Inbound envelope ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Inbound {
    HelloAck(HelloAck),
    Res(Res),
    Evt(Evt),
    Fatal(Fatal),
}
