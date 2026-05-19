# Pixel Agents TUI — Phased Implementation Plan

**Progress as of 2026-05-19:**

- ✅ **Phase 0 — MessageSender refactor** (8 dev-days budget, completed in commits `3d36a3c` + `a6984c4`). All core logic modules decoupled from `vscode`. See CLAUDE.md for current src/ layout.
- ✅ **Phase 1 Day 1 — Daemon scaffold** (commits `ab77a32` + `764da25`). Daemon boots, reads `config.json`, binds UDS socket, writes `daemon.json` with bootId/token/PID, handles SIGTERM/SIGINT cleanly.
- ✅ **Phase 1 Day 2 — Port server/ → daemon/src/hooks/** (commits `47c2288` + `b7ef2f3` + `08f5064`). All hook server + event handler + provider tree moved with planned renames. Hook script discovery chain extended: `PIXEL_AGENTS_HOOK_URL` → `daemon.json` → `server.json`. esbuild path repaired (had been silently no-op'ing). 140 unit tests + 1 E2E test passing.
- ✅ **Phase 1 Day 3-4 — RPC framing on UDS.** `daemon/src/rpc/{framing,wire,connection}.ts` — tag-byte channel mux (0x00 NDJSON, 0x01/0x03 PTY, 0x02 asset blob), 256 KB / 1 MB caps, asset chunking + high-bit-of-tier EOF, streaming decoder. Auth via `daemon.json` token (timing-safe compare). `hello` → `helloAck` with inline stub `WorldSnapshot` (real shape lands Day 5+). 21 Vitest cases including a fuzz scenario across all four channels. 161/161 tests green; live smoke handshake against booted daemon confirms ack on the wire.
- ✅ **Phase 1 Day 5 — Phase-0 modules wired into daemon.** `daemon/tsconfig.json` widened (`rootDir: ".."`) to include `src/{messageSender,terminalRegistry,agentRuntime,types,timerManager,transcriptParser}.ts` directly — no vendoring. Daemon-side impls: `BroadcastSink` (AgentEventSink fanning out to every authed RPC client over UDS, wrapping events into `evt` envelopes with per-topic monotonic `seq`), `DaemonRuntime` (AgentRuntime from boot cwd), `FileStateStore` (AgentStateStore on `~/.pixel-agents/agents.json` with atomic tmp+rename). TerminalRegistry stays `NullTerminalRegistry` until node-pty hosting in Day 13-14. `onAuthenticated` callback on the RPC connection registers each authed socket with the sink. Build emits a `dist/src/package.json {"type":"commonjs"}` scope override so Node 22 ESM can interop with the CJS Phase-0 modules. 9 new Vitest cases; 170/170 total.
- ✅ **Phase 1 Day 7-8 — RPC command catalog.** `daemon/src/rpc/dispatch.ts` introduces `MethodRegistry` (method-name → handler map, throws on duplicate registration), `ConnectionScope` (per-conn: `sessionId`, mutable `subscriptions` Set, raw socket), `DispatchContext` (daemon-wide: writer tag, broadcast sink, agents registry, layout debouncer, mutable `state.layout` / `state.config` refs, `triggerShutdown`), and `ok` / `err` helpers. Methods land under `daemon/src/rpc/methods/`: `layout.get/save/import/export` (save debounced via `LayoutSaveDebouncer.schedule`, broadcasts `layout.changed` with `source:'client'`), `settings.get/set` (defensive per-field patch, broadcasts `settings.updated`), `subscribe` (writes to `scope.subscriptions`), `daemon.shutdown` (defers via `setImmediate` so the `ok` makes it on the wire before close), `agent.list` (reads `AgentsRegistry.forCwd`). The rest (`agent.spawn/close/focus/reassignSeat/adopt`, `pty.input/resize/resync`, `assets.list/requestBlob/addDir/removeDir`, `hooks.toggle`, `layout.setDefault`) registers as `not_yet_supported` with descriptive error messages so the client gets enumerable failure codes rather than `unknown_method`. `BroadcastSink.register(sock, subscriptions)` extended to honour the per-conn filter (empty set = receive all, explicit `"*"` = wildcard). `attachConnection` now takes the registry + dispatch context and routes parsed `Req`s through `MethodRegistry.dispatch`. 21 new Vitest cases (5 layout, 3 settings, 2 subscribe, 1 shutdown, 1 agent.list, 7 gated methods via `it.each`, 2 broadcast filtering); 209/209 total. Live RPC smoke confirms `helloAck` → `settings.get` → `layout.save` (broadcast received) → `agent.spawn` (gated) round-trip.
- ✅ **Phase 1 Day 6 — Persistence ports + writer-tag.** Shared infrastructure under `daemon/src/persistence/`: `writerTag.ts` (atomic tmp+rename, `_writer { processId, bootId }` tagging, type-guarded read with stripped tag in payload) and `watcher.ts` (hybrid `fs.watch` + 2 s polling; own-writes detected by `bootId` match so they never re-emit). Layout, config, and `agents.json` each get their own typed wrapper: `daemon/src/layout/persistence.ts` (read/write/watch + `LayoutSaveDebouncer` 500 ms coalesce for K5), `daemon/src/config/persistence.ts` (defensive coerce, replaces old `daemon/src/config.ts`), `daemon/src/agents/registry.ts` (typed per-cwd `agents.json` shape: `{version:1, agents:{[cwd]:PersistedAgent[]}, _writer}` with `upsert/remove/setCwd/forCwd/cwds`). `FileStateStore` repointed at `~/.pixel-agents/daemon-state.json` (scratchpad), leaving `agents.json` to the typed registry. Server boot loads layout + config, starts watchers, broadcasts `layout.changed` / `settings.updated` evts on external edits via `BroadcastSink`; watcher disposal wired into SIGTERM/SIGINT shutdown. 18 new Vitest cases (4 watcher × external/own/missing/malformed, 6 writer-tag, 5 registry, 3 debouncer); 188/188 total. Live smoke: writing an external-tagged `layout.json` while a client is connected immediately ships a `layout.changed` evt on the wire.
- ✅ **Phase 1 Day 9-10 — AgentEventSink daemon bus.** `AgentEventSink` interface gains `emitTo(agentId, event)`; `WebviewSink` / `NullSink` / `RecordingSink` add trivial impls (recorder also captures targeted events in a `targeted[]` log for assertions). `BroadcastSink.emitTo` filters by per-conn agent subscriptions: clients receive an event for agent N if their subscription set is unfiltered, contains `*`, contains `agent:*`, contains `agent:<N>`, or has no `agent:` entries at all (topic-only filters stay agent-agnostic). Topic and agent dimensions filter independently so a client subscribed to `["agentStatus", "agent:7"]` only gets `agentStatus` events scoped to agent 7. Per-conn high-water-mark backpressure: when `sock.write()` returns `false`, the subscriber is marked paused and subsequent frames go into a bounded ring (`SUBSCRIBER_QUEUE_MAX = 256`, oldest dropped on overflow with `droppedFrames` counter for diagnostics); `'drain'` flushes the queue and fires `onResume`. `register(sock, subs, callbacks?)` exposes optional `onPause` / `onResume` hooks so Day 13-14 PTY pumps can gate `pty.read()` on `BroadcastSink.isPaused(connId)`. 7 new Vitest cases (3 emitTo scope, 1 post-bypasses-agent-filter, 3 backpressure: pause+drain, overflow eviction, dead-socket flush); 216/216 total.
- ✅ **Phase 1 Day 11 — NDJSON logging.** `daemon/src/logging/{logger,retention}.ts`: file-per-UTC-day at `~/.pixel-agents/logs/daemon-YYYY-MM-DD.log` (sync `openSync('a')` + `writeSync`, 0o600), pinned `{ts, level, module, agentId?, ..., msg}` key order, `setLevel()` for runtime updates, optional stderr mirror for `--foreground`. `logLevel` field added to `config/persistence.ts` (default `info`, watcher updates logger on external write). Retention sweep gzips `*.log` >7d → `*.log.gz`, deletes `*.log[.gz]` >30d; runs at boot + every 24 h (unref'd interval). `BroadcastSink.setLogger()` injection replaces its `console.error`. 13 new Vitest cases; 229/229 total. Live smoke: boot/shutdown round-trip writes valid NDJSON.
- ✅ **Phase 1 Day 12 — Hook integration test.** New `daemon/src/hookHost/{server,bridge}.ts`: minimal 127.0.0.1 hook HTTP server (port 0, Bearer auth = daemon's UDS token, `/api/hooks/:providerId`) + `DaemonHookBridge` (sessionId→agentId map, per-agent toolId stack, normalizes SessionStart/PreToolUse/PostToolUse/Stop/UserPromptSubmit/PermissionRequest/Notification to `agent.created`/`agent.toolStart`/`agent.toolDone`/`agent.statusChanged`/`agent.exited` topics). `daemon.json` now publishes `hookPort`. `PIXEL_AGENTS_HOME` env override in `paths.ts` for test isolation. Integration test boots real BroadcastSink + bridge + HTTP server + UDS, drives end-to-end via POSTs + UDS subscribe. 3 new Vitest cases; 232/232 total. Live smoke: daemon.json with hookPort, auth'd POSTs return `ok`.
- ✅ **Phase 1 Day 13-14 — Agent spawn + PTY hosting.** `daemon/src/agents/ptyHost.ts` (`PtyHost` wrapper: constructor injects `spawn` for tests, exposes `write`/`resize`/`kill`, fires `onData`/`onExit`). `daemon/src/agents/liveAgents.ts` (`LiveAgents` Map<id, LiveAgent>). `BroadcastSink.broadcastPty(agentId, bytes)` ships 0x01 frames to per-agent subscribers; auto-splits above 1 MB cap. `agent.spawn` (UUID sessionId, alloc agentId, `claude --session-id`, persist, emit `agent.created`), `agent.close` (SIGTERM + 2s SIGKILL, depersist), `pty.input` (base64 → write), `pty.resize`. `node-pty@^1.2.0-beta.13` dep. 12 new Vitest cases including `/bin/cat` PTY round-trip; 245/245 total. Live smoke: daemon boots, publishes daemon.json, accepts SIGTERM cleanly.
- ✅ **Phase 1 Day 15-16 — `--resume` revival.** `daemon/src/agents/resume.ts` (`reviveAgentsOnBoot` iterates `agents.json` per-cwd: JSONL liveness gate (exists + mtime <30d), `claude --resume <sessionId>` via PtyHost, 3s health check via `Promise.race`). Seven failure paths: JSONL missing/stale → drop + log; exit 127 → `agent.spawnFailed { reason: "claude_missing" }`, keep; exit 2 + mismatch message → `agent.spawnFailed { reason: "claude_upgraded" }`, keep; other early exit → drop; hangs → keep PTY alive. Successful revival emits `agent.created { isResumed: true }` + refreshes `lastSeenAt`. `classifyExit()` helper also wired into live `agent.spawn` `onExit`. Clean user-exit removes from persistence. Revival fires background after socket open. 10 new Vitest cases; 255/255 total. Build clean.
- ✅ **Phase 2 pre-work — JSONL polling port.** `daemon/src/agents/jsonlPoller.ts` (`JsonlPoller`: per-agent 500 ms `setInterval`, `start/stop/stopAll`, inlines `readNewLines` (64 KB cap, lineBuffer for partial lines), delegates to `processTranscriptLine`. `seedOffset=0` for fresh spawns, `stat.size` for revivals. `markHookDelivered(agentId)` suppresses heuristic timers when hooks active. `DaemonHookBridge.setHookDeliveredCallback` fires on PreToolUse/PostToolUse/Stop/Notification/SessionEnd. Poller wired in `agent.spawn/close` and `reviveAgentsOnBoot`. `stopAll()` in shutdown. 9 new Vitest cases using `vi.useFakeTimers()` + real `fs.appendFileSync`; 264/264 total.
- ✅ **Phase 2 pre-work — Daemon supervisor.** `daemon/src/supervisor.ts` (`installSupervisor({ nodePath?, scriptPath?, platform? })`): generates systemd unit / launchd plist / Windows Scheduled Task XML for on-failure-only restart. `--install-supervisor` flag in `server.ts` early-exits, prints config path + activate command, never auto-enables. Fixed pre-existing build bug: `dist/daemon/src/hooks/package.json {"type":"commonjs"}` now written by build step. 9 new Vitest cases; 273/273 total.
- ✅ **Phase 2 Day 8 — Event loop skeleton.** `client/src/tui.rs`: `Tui` RAII guard (enters alternate screen + raw mode in `new()`; `draw<F>()` delegates to `Terminal::draw`; `Drop` calls `disable_raw_mode` + `LeaveAlternateScreen`). `client/src/app.rs`: `run(conn: DaemonConn)` — `tokio::select!` over 4 arms: daemon `recv_frame()`, `EventStream::next()` (crossterm event stream), `tick.tick()` (17 ms / ~60 fps, `MissedTickBehavior::Skip`), `Sigwinch::recv()`. Quit on `q` or `Ctrl-C`. `render()` placeholder: `Office` bordered block + bottom toolbar (`+ Agent / Layout / Settings / q/Ctrl-C`). `Sigwinch` abstraction: `#[cfg(unix)]` wraps `tokio::signal::unix::signal(SIGWINCH)`, `#[cfg(not(unix))]` uses `std::future::pending::<()>()` — required because `#[cfg]` attrs cannot appear inside `tokio::select!` arms. `DaemonConn` struct: private `reader`/`writer`/`decoder` fields; `#[allow(dead_code)]` on struct + impl for Day 8. 36/36 tests pass, zero warnings.
- ✅ **Phase 2 Day 5-7 — Capability detection + pre-app input queue.** `client/src/caps/`: `probe.rs` (DA1/Kitty/iTerm2/CSI-14t probe byte sequences; `parse_replies()` feeds vte for CSI+OSC + manual APC scan since vte 0.15 discards APC body; `is_native_kitty()` env heuristic; `in_multiplexer()`). `cache.rs` (JSON serde roundtrip, 8-env-var key, 7-day TTL, atomic tmp+rename). `mod.rs` (`detect()`: env override → cache hit → non-tty guard → live probe; `cap_at_most_sixel()` demotes Kitty/iTerm2 to Sixel when inside multiplexer; `PIXEL_AGENTS_TIER` string override). `raw_mode.rs` (`RawModeGuard` RAII, `Drop` disables raw mode). `input_queue.rs` (`InputQueue` push/drain backed by `BytesMut`; bytes during 150ms probe window dropped by design). `connect()` refactored to accept `ClientCapabilities` param; construction lifted to `main.rs`. 15 new unit tests (probe roundtrips + mux/kitty/iterm2/cell-size, cache serde + TTL, queue push/drain); 36/36 total. Smoke-tested: `PIXEL_AGENTS_TIER=t4 ./pa-tui` → `connected`. Zero warnings.
- ✅ **Phase 2 Day 3-4 — Streaming framing decoder + encoder.** `client/src/daemon/framing.rs`: `FrameDecoder` (streaming `push`/`drain`, poisoned-on-error), all 4 frame types (`Ndjson`, `PtyOut`, `PtyIn`, `Asset`), `encode_ndjson<T: Serialize>()` + `encode_pty_in()` public encoders, `encode_pty_out()` + `encode_asset()` test-only encoders. Asset tier byte: high bit = `is_final`, low 7 bits = tier number. `connection.rs` updated: send path uses `encode_ndjson`, recv path replaces BufReader with `recv_one_frame` loop feeding `FrameDecoder`. 21 unit tests: roundtrip + byte-by-byte for every frame type, mixed-type sequencing, split-header/split-payload resumption, oversized-frame and unknown-tag errors, poison propagation. Smoke-tested: `cargo run` → `connected`. 21/21 tests pass, zero warnings.
- ✅ **Phase 2 Day 1-2 — Rust TUI client scaffold + UDS handshake.** `client/` Cargo workspace (`pa-tui` bin). Pinned deps: `ratatui 0.30`, `ratatui-crossterm 0.1`, `crossterm 0.29`, `tokio 1`, `serde/serde_json 1`, `bytes 1`, `vte 0.15`, `tachyonfx 0.25`, `arboard 3`, `directories 6`, `image 0.25`, `anyhow 1`; `wezterm-term` deferred to Phase 3. Module tree: `daemon/{mod,wire,discovery,connection}.rs`. `wire.rs` ports all TS wire types with correct `#[serde(tag="kind")]` internally-tagged enum. `discovery.rs` reads `~/.pixel-agents/daemon.json`. `connection.rs`: `tokio::net::UnixStream`, NDJSON frame send (`[0x00][json][0x0a]`), recv until `\n` with BufReader, bootId pinning. `ClientCapabilities` has all required fields (`cols`, `rows`, `cellPx`, `bracketedPaste`, `mouse`) + `clientVersion`. Smoke-tested: `cargo run` → `connected: daemon 0.0.1 boot=<8chars> session=<8chars>`. 273/273 daemon Vitest tests pass.

## 1. Overview

**Total scope.** Port the Pixel Agents VS Code extension into a daemon + Rust TUI client architecture, hitting every MVP item in `docs/tui-parity-checklist.md` while leaving every Full item architecturally non-blocked. The codebase split is: (a) refactor the existing TS extension in-place behind a `MessageSender`/`AgentEventSink` interface (Phase 0); (b) lift it into a standalone `daemon/` Node.js 22 LTS package; (c) build a new Rust 1.79+ Ratatui 0.30 client from scratch, porting the pure logic in `webview-ui/src/office/engine/` 1:1. See architecture §1 for the four headline decisions.

**Recommended team size.** Two engineers, full-time. Engineer A is the "TS/daemon lead" (owns Phases 0, 1, 3-asset-side, 6, 7-Node, 8-server). Engineer B is the "Rust/TUI lead" (owns Phases 2, 3-render-side, 4, 5, 7-cargo, 8-client). Phases 3 onward parallelize cleanly.

**Calendar estimate (2-engineer team, dev-days).**

| Phase                                   | Dev-days         | Wall-clock weeks       |
| --------------------------------------- | ---------------- | ---------------------- |
| 0 — MessageSender refactor              | 8                | 1.0                    |
| 1 — Daemon foundation                   | 16               | 2.0                    |
| 2 — TUI client foundation               | 14               | 1.75                   |
| 3 — Office rendering                    | 22               | 2.75                   |
| 4 — PTY hosting                         | 12               | 1.5                    |
| 5 — Layout editor                       | 14               | 1.75                   |
| 6 — Multi-client + persistence sync     | 8                | 1.0                    |
| 7 — Polish / cross-platform / packaging | 14               | 1.75                   |
| 8 — Testing & release                   | 12               | 1.5                    |
| **Total**                               | **120 dev-days** | **~15 weeks calendar** |

Phases 1 and 2 can start in parallel after Phase 0 lands. Phase 4 (PTY) and Phase 5 (editor) parallelize against the back half of Phase 3. **MVP total: 80 dev-days (~10 weeks calendar).** Full plan (all phases through 8): 120 dev-days.

---

## 2. Phase 0 — MessageSender refactor

**Goal.** Decouple `src/transcriptParser.ts`, `src/timerManager.ts`, `src/fileWatcher.ts`, and `src/agentManager.ts` from `vscode.Webview` and `vscode.window.*` so the same source files can run unchanged inside the future daemon. (Architecture §11.)

**Parity items covered.** Indirect prerequisite for every C/D item; no parity items shipped directly. Phase 0 keeps the existing VS Code extension green.

**Concrete tasks.**

1. **Day 1** — Create `src/messageSender.ts` with `AgentEvent` discriminated union, `AgentEventSink` interface, and `WebviewSink` implementation. Lift the existing message shape adapter into `eventToWebviewMessage()`. Unit tests covering every `kind` round-trip.
2. **Day 1-2** — Refactor `src/transcriptParser.ts` (3 `vscode` lines): replace `webview: vscode.Webview | undefined` with `sink: AgentEventSink`. Tests stay green.
3. **Day 2** — Refactor `src/timerManager.ts` (4 lines): identical pattern.
4. **Day 3-4** — Introduce `TerminalRegistry` interface with VS Code impl wrapping `vscode.window.activeTerminal` and `vscode.window.terminals`. Tests for adoption path.
5. **Day 4-5** — Refactor `src/fileWatcher.ts` (19 lines): inject `TerminalRegistry`. **Write fixture-based regression tests for the dual-mode session detection BEFORE refactoring** to pin current behavior.
6. **Day 5-6** — Introduce `AgentRuntime` + `AgentStateStore` interfaces plus VS Code impls. `AgentRuntime` wraps `vscode.window.createTerminal`; `AgentStateStore` wraps `context.workspaceState`.
7. **Day 6-7** — Refactor `src/agentManager.ts` (15 lines): inject runtime + store. Update `src/PixelAgentsViewProvider.ts` to construct VS Code impls.
8. **Day 7-8** — Smoke test + tighten gates: full Playwright e2e, parity-checklist manual walkthrough.

**Exit criteria.**

- `grep -nE "vscode" src/{transcriptParser,fileWatcher,agentManager,timerManager}.ts` returns zero matches (except one scoped `import type` for `ExtensionContext` with `// extension-only` comment).
- All unit suites pass (`npm test`).
- Playwright e2e passes (`npm run e2e`).
- Manual smoke: open extension dev host, spawn 2 agents, run `/clear`, restart workspace — restoration works.

**Risks.**

- _Hidden coupling in `fileWatcher.ts` adoption path_ → fixture tests before refactor.
- _`AgentStateStore` migration of `workspaceState` semantics_ → VS Code impl is verbatim; no data model change.

**Calendar estimate.** 8 dev-days.

---

## 3. Phase 1 — Daemon foundation

**Goal.** Stand up `daemon/` as a runnable Node.js 22 LTS package that boots, listens on a Unix domain socket (or named pipe on Windows), serves the NDJSON+binary wire protocol, and reuses every Phase-0-refactored module. The hook HTTP server in `server/` is moved in, not rewritten. (Architecture §4, §5, §10, §15.)

**Parity items covered.** A1, A2, A3, A4 (foundation; `--resume` revival in Phase 1 days 15-16), C1-C11 (logic preserved), D1-D5 (port verbatim), J1, K1-K3, K7, L4.

**Concrete tasks.**

1. **Day 1** — Scaffold `daemon/`. Boot sequence: read `~/.pixel-agents/config.json` → bind socket → write `~/.pixel-agents/daemon.json` with `bootId` UUIDv4 + auth token + PID. Atomic write helpers.
2. **Day 2** — Port `server/` verbatim into `daemon/src/hooks/`. Hook script discovery: `daemon.json` → `server.json` → `$PIXEL_AGENTS_HOOK_URL`. Reuse all `server/__tests__/`.
3. **Day 3-4** — RPC + framing in `daemon/src/rpc/`. Channel multiplex: 0x00 NDJSON, 0x01 PTY out, 0x02 asset blob, 0x03 PTY in (>64 KB). NDJSON line cap 256 KB; binary frame cap 1 MB.
4. **Day 5** — Port Phase-0 modules into `daemon/src/watching/` and `daemon/src/agents/`. Inject daemon impls of `AgentRuntime` (node-pty wrapper), `AgentStateStore`, `TerminalRegistry`.
5. **Day 6** — Persistence ports. `layoutPersistence.ts` → daemon with `_writer` tag. `configPersistence.ts` → daemon. `assetLoader.ts` → daemon. `agents.json` schema (per-cwd indexed).
6. **Day 7-8** — RPC command catalog: `hello`, `helloAck` (with inline `WorldSnapshot`), `agent.list/spawn/close/reassignSeat`, `layout.get/save`, `assets.list`, `settings.get/set`, `daemon.shutdown`, `subscribe`. Vitest per method.
7. **Day 9-10** — `AgentEventSink` daemon bus. `broadcast()` walks connected clients; `emitTo(agentId, ...)` for per-agent scope. Socket high-water-mark backpressure pauses PTY pumps.
8. **Day 11** — NDJSON logging to `~/.pixel-agents/logs/daemon-YYYY-MM-DD.log`, rotated daily, gz 7d, delete 30d.
9. **Day 12** — Hook integration test. End-to-end: real `claude` → hook script → daemon → mock client sees `agent.toolStart`.
10. **Day 13-14** — Agent spawn + JSONL polling live. PTY data streams over 0x01 (full PTY hosting in Phase 4).
11. **Day 15-16** — `--resume` revival. On boot, iterate `agents.json`, JSONL liveness gate, `claude --resume <id>` spawn, 3 s health check. Seven failure paths per Architecture §16.

**Exit criteria.**

- `pixel-agents --daemon --foreground` boots in <500 ms, writes `daemon.json`, listens on socket.
- `nc -U ~/.pixel-agents/socket` + `hello` returns valid `helloAck` with inline `WorldSnapshot`.
- Existing `server/__tests__/` all pass when moved.
- Scripted client can spawn agent, see `agent.created`, observe `agent.toolStart` from real claude Write.
- `agents.json` round-trip via daemon restart preserves live-JSONL agents.

**Risks.**

- _`node-pty 1.2.0-beta.13` API drift_ → pin exact version; abstract `AgentRuntime.spawnAgent`.
- _NDJSON framing bugs at socket boundaries_ → fuzz tests for random chunked reads.
- _Cooperative-with-extension regression_ → behind config flag in Phase 1; turn on in Phase 6.

**Calendar estimate.** 16 dev-days. Engineer A leads.

---

## 4. Phase 2 — TUI client foundation

**Goal.** Bring up `client/` as a Rust 1.79+ Cargo crate that connects to the daemon, completes the handshake, renders an empty Ratatui shell with the bottom toolbar, and switches between Office / PtyAgent / Editor / Modal focus modes. Capability detection + fallback ladder lands here. (Architecture §6, §7.)

**Parity items covered.** Q1/Q2 (Linux + macOS first-class), R1/R2 (frame budget framework), N1 (bottom toolbar).

**Concrete tasks.**

1. **Day 1** — Cargo workspace. Pin: `ratatui 0.30`, `ratatui-crossterm 0.1`, `crossterm 0.29`, `tokio 1`, `serde/serde_json 1`, `bytes 1`, `vte 0.15`, `tachyonfx 0.25`, `arboard 3`, `directories 6`, `image 0.25`, `anyhow 1`. `wezterm-term` deferred to Phase 3 (not on crates.io; vendor at Phase 4).
2. **Day 2** — Socket connect + `hello` handshake. `bootId` pinning; reconnect on bootId change.
3. **Day 3-4** — Framing decoder. Tag-byte dispatch. Tests against recorded daemon byte streams.
4. **Day 5-6** — Capability detection pipeline. Pre-app input drain thread (vte 0.13). Parallel probes (DA1, Kitty, iTerm2, unicode placeholder, `CSI 14 t`) with 150 ms aggregate timeout. `PIXEL_AGENTS_TIER` env override. Cache `~/.pixel-agents/capabilities-cache.json` 7-day TTL.
5. **Day 7** — Pre-app input queue. Buffer drained bytes during probe, feed into main loop after raw mode entry.
6. **Day 8** — Event loop skeleton (tokio::select! over socket / crossterm / 16.66 ms timer / SIGWINCH).
7. **Day 9** — Focus state machine. Tab / Ctrl+Alt+O / Ctrl+Alt+L. Bracketed paste per focus mode. Keymap from `~/.pixel-agents/keymap.toml`.
8. **Day 10** — Ratatui chrome. Bottom toolbar `+ Agent / Layout / Settings`. Top-right `ZoomControls`. SGR mouse hit-tested on toolbar.
9. **Day 11-12** — `agent.list` + `agent.spawn`. `+ Agent` button → spawn RPC. Per-agent status row.
10. **Day 13-14** — Reconnect logic + supervisor handoff. `read() == 0` → "Reconnecting…" → retry 3 s with two probes at 250 ms / 1 s; fork detached daemon if needed.

**Exit criteria.**

- `cargo build --release` produces `pixel-agents-tui` binary.
- Handshake completes; capability detection <200 ms cold or <5 ms cached; toolbar renders.
- `PIXEL_AGENTS_TIER=truecolor` forces T4.
- Kill daemon; client reconnects when it returns.
- Tab toggles Office ↔ PtyAgent; in PtyAgent, Tab passes to PTY.
- `cargo test --test caps_test` per-terminal fixtures map to expected tiers.

**Risks.**

- _Ratatui 0.30 modular workspace breaking changes_ → pin minor; `Cargo.lock`.
- _Probe replies truncated under tmux without passthrough_ → `$TMUX` short-circuit drops to T3.
- _Pre-app input queue races crossterm raw-mode entry_ → `tokio::sync::Notify` between drain-stop and raw-mode-on.

**Calendar estimate.** 14 dev-days. Engineer B leads.

---

## 5. Phase 3 — Office rendering

**Goal.** Port the visual office from `webview-ui/src/office/` 1:1 into Rust, with daemon-side sprite cache feeding per-tier encoders. Animation runs from client's own `OfficeState` FSM seeded by `worldSeed` (Architecture §5, §8, §13).

**Parity items covered.** F1, F2, F3, F4, F5, F6 (T1-K/T1-O/T2/T3 ✓; T4-T6 ⚠), G1, G2, G3, H1, H2, H3, J1, J2 (MVP); F7-F14, G4-G7, H4-H13, J3, J4 (Full).

**Concrete tasks.**

1. **Day 1-2** — Daemon asset pipeline. Enumerate bundled + external dirs. Build rotation/state groups. PNG→RGBA via `pngjs`. Emit `assets.updated` event (chokidar 250 ms debounce).
2. **Day 2-3** — `assets.requestBlob` RPC + asset blob (0x02) channel. Chunked with EOF bit. Tier blobs generated lazily.
3. **Day 3** — `kittyImageId` allocation: sha1-keyed lazy + memoization for shared kittyImageId across spawns.
4. **Day 4-7** — Port pure FSM/engine code to Rust:
   - `engine/officeState.ts` → `client/src/office/state.rs`
   - `engine/characters.ts` → `client/src/office/characters.rs`
   - `engine/gameLoop.ts` → `client/src/office/loop.rs` (rAF → `tokio::time::interval(16.66 ms)`)
   - `engine/matrixEffect.ts` → `client/src/render/matrix.rs`
   - `layout/*.ts`, `colorize.ts`, `toolUtils.ts`, `floorTiles.ts`, `wallTiles.ts`
   - **Shared JSON fixtures** for parity (consumed by TS + Rust tests).
5. **Day 8-9** — `worldSeed` determinism. Client wander RNG seeded with `worldSeed XOR agentId`. Verify two clients produce identical positions tick-for-tick. `cargo test --test fsm_parity`.
6. **Day 10-11** — Tier T1-K (Kitty unicode placeholders). `\x1b_Ga=T,i=<id>,U=1,c=1,r=1,...`. Pixel-exact via `X=`/`Y=` sub-cell offsets.
7. **Day 12** — Tier T1-O. Non-virtual placement (`a=T` without `U=1`).
8. **Day 13** — Tier T2 (iTerm2 inline). OSC 1337 base64 PNG. Quadrant-dirty mitigation.
9. **Day 14** — Tier T3 (Sixel). DCS Sixel pre-quantized per sprite/zoom. Frame budget 30 ms (15 fps on xterm).
10. **Day 15-16** — Tiers T4-T6 (half-block / block / braille). Horizontal pixel doubling documented.
11. **Day 17** — Z-sort + render pipeline. FSM tick → static layers → z-sorted entities → back-buffer → chrome → diff → Ratatui emit.
12. **Day 18** — Character sprites + palette. Load `char_0.png`–`char_5.png`. Hue-shifted lazy. `pickDiversePalette()` ported.
13. **Day 19** — Speech bubbles (F7). Permission amber, waiting green 2s fade.
14. **Day 20** — Matrix spawn/despawn (F10). Phase locked to event `t0`. `tachyonfx 0.24` overlay.
15. **Day 21** — Tool overlay + selection outline (F8, F9). White outline via cached outline sprites.
16. **Day 22** — Snapshot tests per tier × scene matrix with `insta`.

**Exit criteria.**

- All MVP F items render in T1-K with pixel-perfect fidelity at zoom 1-10.
- All half-block tiers render with documented horizontal-doubling stretch.
- `cargo test --test fsm_parity` confirms deterministic FSM across two clients on same `worldSeed`.
- Frame budgets per §19 met: T1-K ≤16 ms, T3-foot ≤33 ms, T4 ≤8 ms.
- All 6 palettes + hue shifts visible.
- `insta` snapshots committed for tier × scene matrix.
- Manual visual diff against webview at zoom 2 — identical positions/sprites/animations.

**Risks.**

- _Pixel-perfect at sub-cell positions doesn't survive resize on some terminals_ → per-tier quirk-list; T1-K/T1-O probe split.
- _Sixel encoder performance unacceptable_ → pre-quantize at asset-load; fall back to T4 if frame budget breached 3× in a row.
- _Parity drift between TS engine and Rust port_ → shared JSON fixtures.

**Calendar estimate.** 22 dev-days. Engineer B leads rendering + FSM port; Engineer A owns daemon asset pipeline + tier-blob lazy generation.

---

## 6. Phase 4 — PTY hosting

**Goal.** Daemon spawns and hosts PTY, streams bytes over binary mux, client feeds them through `PtyByteTap` then `wezterm-term::Terminal`, user can type, resize, paste, mouse, scroll back. (Architecture §9.)

**Parity items covered.** B1, B2, B3, B4 (MVP); B5, B6, B7 (Full).

**Concrete tasks.**

1. **Day 1** — `daemon/src/agents/ptyHost.ts`. `node-pty` spawn with `encoding: null`. `pty.onData` → broadcast over 0x01. `pty.onExit` → emit `agent.exited`.
2. **Day 2** — Scrollback: 256 KB bounded ring per agent. Pause/resume on socket high-water mark.
3. **Day 3-4** — Client `wezterm-term` integration. Per-agent `Terminal`. `term.advance_bytes(tap.intercept(bytes))`. Grid rendered when focused.
4. **Day 4-5** — `PtyByteTap` impl. `KittyPassthroughTap` verbatim stdout when tier ∈ {T1-K, T1-O} ∧ focused. Otherwise strip Kitty APC + iTerm2 OSC 1337 + Sixel DCS via state machine. Tests: mixed streams, pixel-perfect passthrough vs strip.
5. **Day 6** — Focus arbitration. `agent.focus` RPC stores `focusedClient[id]`; emits `agent.focusLost` to prior owner. PTY resize follows focused client (debounced 250 ms). Non-focused clients render scaled-down preview.
6. **Day 7** — Resize + SIGWINCH. `pty.resize` RPC → `pty.resize(cols, rows)`.
7. **Day 8** — Bracketed paste + mouse modes. PtyAgent focus: reconstruct full BPM, send as `pty.input` or 0x03.
8. **Day 9** — Scrollback display (B4). Show `wezterm-term` Screen + history when PTY focused. PgUp/PgDn/Shift+arrow.
9. **Day 10** — `pty.resync` (force redraw replay) when client ring buffer overflows.
10. **Day 11** — Click-to-focus character → PTY focus (B5). Sub-agent click focuses parent (B6); negative IDs mapped to parent.
11. **Day 12** — Graceful PTY death (B7). `onExit` → `agent.exited` → despawn matrix → remove from `agents.json`. 30 s safety check.

**Exit criteria.**

- Spawn 3 agents; focus each via click; type into each; observe output; resize → PTY grid resizes with focused client.
- Kitty + `bat`-via-claude → image escapes pass through.
- Alacritty (T4) → same escapes stripped, no garbage.
- `cargo test --test pty_tap` confirms `PtyByteTap` handles split escapes across `intercept` calls.
- Focus storm test: rapid A↔B; only last winner triggers resize; daemon emits `agent.focusLost`.

**Risks.**

- _`wezterm-term::Terminal` API changes between 0.22 minors_ → pin `=0.22.0`; vendor `tattoy-wezterm-term`.
- _Image-escape passthrough breaks on alt display_ → only passthrough when `tier.supports_kitty_passthrough() && focused`.
- _PTY-input ordering across NDJSON + binary_ → documented no-cross-channel-ordering; kernel TTY arbitrates.

**Calendar estimate.** 12 dev-days.

---

## 7. Phase 5 — Layout editor

**Goal.** Keyboard-first layout editor in Ratatui covering every Section I parity item. Edits via `layout.save` RPC; writer-tag prevents echo. (Architecture §12, §16.)

**Parity items covered.** I1-I15 (all Full).

**Concrete tasks.**

1. **Day 1** — Editor enter/exit (I1, I12). `L` toggle. Multi-stage Esc.
2. **Day 2** — Brush cursor. Arrow keys, Shift+arrow (5-tile), Space/Enter apply.
3. **Day 3** — Tool palette (I2). Right-side panel (`P`); 1-7 shortcuts.
4. **Day 4** — Furniture catalog modal. Per-rotationGroup list; category tabs; miniature via Kitty placement.
5. **Day 5** — Ghost preview (I5). Green/red validity using ported `canPlaceFurniture()`.
6. **Day 6** — Place / Remove / Rotate / Toggle-state (I6). R rotates ghost; T toggles state.
7. **Day 7** — Drag-paint (I4). SGR mouse path painted; keyboard Shift+arrow drag-paint mode.
8. **Day 8** — Drag-to-move selected furniture (I7).
9. **Day 9** — Delete + Rotate buttons (I8). EditActionBar: Undo (U), Redo (Y), Save (S), Reset (Z); per-furniture Rotate (R), Delete (D).
10. **Day 10** — HSBC sliders (I3). Per-element panel. H/L ±1, Shift ±10. Colorize toggle (C). Single-undo-per-burst via 500 ms idle.
11. **Day 11** — Surface-item priority + eyedroppers (I9).
12. **Day 12** — Erase to VOID (I13). Right-click also erases.
13. **Day 13** — Grid expansion (I14, I15). Cursor + Shift+arrow past edge → dashed ghost outline → Space expands. Max 64×64.
14. **Day 14** — Undo/Redo + Save/Reset. 50-level per-client. `layout.save` with `writerTag`. Conflict → `STALE_LAYOUT` → "Reload?" toast.

**Exit criteria.**

- All I1-I15 pass manual walkthrough.
- Editor works fully from keyboard.
- Mouse-augmented in SGR-mouse terminals.
- Concurrent edit test: A and B both edit; A saves; B saves; daemon last-save-wins; writer-tag suppresses echo to A.
- `insta` snapshots for editor states.

**Risks.**

- _HSBC slider mouse hit-test imprecise across tiers_ → SGR mouse cell-precision; Shift for ±10.
- _Drag-paint UX confusing without visible cursor in slow terminals_ → bottom-left "PAINTING" indicator.

**Calendar estimate.** 14 dev-days. Engineer B leads.

---

## 8. Phase 6 — Multi-client + persistence sync

**Goal.** Multi-client cooperation with file-watcher-driven cross-client propagation, writer-tag conflict resolution, layout file watcher. (Architecture §16, §10.)

**Parity items covered.** L1, L2, L3, L4 (Full); K4, K5, K6, K7, K8, K9 (Full).

**Concrete tasks.**

1. **Day 1** — Multi-client socket. `MAX_CLIENTS = 8`. Per-connection `clientId`.
2. **Day 2** — Per-client subscription scoping. `subscribe { topics: [...] }`.
3. **Day 3** — Layout file watcher (K4). `fs.watch` + 2 s polling. Parse `_writer.bootId`; match → ignore; mismatch → broadcast.
4. **Day 4** — Writer-tag round-trip (K5). Round-trip via daemon restart preserves correct suppression.
5. **Day 5** — Conflict resolution (K6). Any dirty client → suppress external `layout.changed` for 10 s; when all clean, apply + broadcast.
6. **Day 6** — Export / Import (K8) + Set Default (K9).
7. **Day 7** — Per-client independent viewports (L3). Each client tracks own zoom/pan/focus/hover/selection/undo.
8. **Day 8** — Cooperative-with-extension mode (CRIT-5). Re-enable. Daemon detects existing `server.json` → boot without installing hooks. Hook script tries `daemon.json` first.

**Exit criteria.**

- 3 clients; edit in A; B + C receive update.
- `_writer.bootId` round-trip suppresses daemon echo.
- Manual `vim` edit to `layout.json` broadcasts to all clients.
- VS Code extension + daemon coexist; spawn in either, focus in client.
- L1-L4, K4-K9 all pass.

**Risks.**

- _`fs.watch` unreliable on Windows_ → 2 s polling already in design.
- _Writer-tag collision_ → UUIDv4 collision negligible.

**Calendar estimate.** 8 dev-days.

---

## 9. Phase 7 — Polish / cross-platform / packaging

**Goal.** Single-command install on Linux/macOS/Windows. Supervisor configs. Audio/notifications. Settings, changelog, tooltip polish. (Architecture §17, §4, §15, §20.)

**Parity items covered.** O1, O2, O3, O4, Q1, Q2, Q3, Q4, M1, M2, M3, N2, N3 (⚠), N4, N5, N6 (⚠).

**Concrete tasks.**

1. **Day 1** — npm package. `pixel-agents` with `dist/daemon/`, `dist/hooks/claude-hook.js`, `bin/pixel-agents` launcher. Postinstall fetches Rust binary tarball, sha256-verified against shipped manifest.
2. **Day 2** — Cargo distribution. `pixel-agents-tui` on crates.io (client-only; daemon via npm).
3. **Day 3** — Supervisor scripts. `share/supervisors/{systemd.service, launchd.plist, scheduled-task.xml}`. Postinstall prints activation command — never auto-enable.
4. **Day 4** — Launcher binary (Rust). Reads both `daemon.json` and `server.json`. Cold-start 3 s retry with one-shot daemon respawn.
5. **Day 5** — Audio cascade. Linux: pw-play → paplay → aplay → bell. macOS: afplay → osascript. Windows: PowerShell SoundPlayer (warm pool). Notification cascade: notify-send / terminal-notifier / BurntToast.
6. **Day 6** — Settings: `notificationsEnabled`, `soundEnabled` toggles. `--sound-cmd` daemon flag.
7. **Day 7** — Windows ConPTY (Q3). node-pty Windows build. Named pipe `\\.\pipe\pixel-agents-<sha1(user@host)>`. Crossterm `ENABLE_VIRTUAL_TERMINAL_PROCESSING`.
8. **Day 8** — macOS launchd (Q2). `xcode-select --install` postinstall check.
9. **Day 9** — tmux / zellij / screen probes (Q4). Warning toast if passthrough off.
10. **Day 10** — Settings modal (N2). Sound, hooks toggle, debug, asset directories, export/import.
11. **Day 11** — Info modal + changelog (N3 ⚠). Per-tier GIF behavior.
12. **Day 12** — First-run tooltip (N4) + debug overlay (N5). Tooltip with "View more"; dismiss persisted. Debug subscribes to `daemon.log`.
13. **Day 13** — FS Pixel Sans body font (N6 ⚠). Sprite-rendered headings in image tiers; T5/T6 ANSI bold + color.
14. **Day 14** — Auto-update channel (O4). `autoUpdate: "off" | "check"`. Banner only. No in-place upgrade.

**Exit criteria.**

- `npm install -g pixel-agents` on clean Linux/macOS/Windows boxes → boots and renders.
- `pixel-agents --install-supervisor` installs systemd/launchd/Scheduled Task. Reboot → daemon restarts.
- Sound chime fires on waiting bubble; notification fires when foreground client not focused on agent.
- All O1-O4, Q1-Q4, M1-M3, N1-N6 pass.

**Risks.**

- _sha256 mismatch on postinstall tarball_ → canonical sha in npm tarball, not remote URL.
- _Windows named-pipe ACL_ → SACL restricted to current user.
- _PowerShell startup latency ~150 ms_ → warm-pooled child process with stdin pump.

**Calendar estimate.** 14 dev-days.

---

## 10. Phase 8 — Testing & release

**Goal.** Drive testing pyramid from unit through E2E across terminal compatibility matrix. Ship 1.0. (Architecture §18.)

**Parity items covered.** P1, P2, P3, P4, P5 (all Full).

**Concrete tasks.**

1. **Day 1** — Daemon unit-test pass. RPC, framing, auth, agents lifecycle including `--resume` failure paths, hooks, transcript parser, timers, layout, conflict resolution.
2. **Day 2** — Client cargo-test pass. Capability detection fixtures, rendering snapshots per tier × scene, editor ops + undo invariants, FSM determinism via `worldSeed`.
3. **Day 3** — Pathfinding parity. Both TS + Rust consume `tests/fixtures/pathfinding-fixtures.json`; identical BFS outputs.
4. **Day 4** — `insta` snapshot review for all tiers × scenes. Pinned dimensions 80×24, 120×40, 200×60.
5. **Day 5-6** — E2E with real `claude`. Spawn via daemon; scripted prompt; assert event sequence within `turn_duration` window. `claude-mock` binary for hermetic CI.
6. **Day 7-10** — Terminal compatibility matrix. Kitty 0.36+, Ghostty 1.3+, WezTerm nightly, Alacritty 0.14+, foot 1.21+, xterm -ti vt340, gnome-terminal, Windows Terminal 1.22+, Apple Terminal, tmux 3.4 + Kitty/Ghostty/WezTerm. Automated via `expect` + `tmux capture-pane`.
7. **Day 11** — Supervisor smoke tests. Install scripts pass `--check`; kill daemon → restart.
8. **Day 12** — Release engineering. Tag v1.0.0; `npm publish`; GitHub Release with Rust binaries for linux-x64-glibc/musl, linux-arm64-musl, darwin-x64/arm64, windows-x64. Manifest sha256s committed.

**Exit criteria.**

- All P1-P5 pass; CI green.
- `docs/tui-terminals.md` lists each tested terminal × tier pass/fail.
- `npm install -g pixel-agents@1.0.0` works on three reference OSes.
- Manual smoke: 5 agents, focus each, edit layout, restart daemon → all 5 revive via `--resume`.

**Risks.**

- _Real `claude` flakiness in CI_ → `claude-mock` replays prerecorded JSONL.
- _Terminal matrix containers drift_ → pin Docker tags.
- _Windows runner availability_ → PR-only Windows runners; nightly main matrix.

**Calendar estimate.** 12 dev-days.

---

## MVP definition

**MVP = Phases 0 + 1 + 2 + 3 (MVP slice) + 4 (MVP slice) + Phase 7 minimum subset + Phase 8 unit + E2E only.**

Exact parity items required:

- All **A1-A4** (lifecycle)
- All **B1-B4** (PTY)
- All **C1-C4** (JSONL polling + tool tracking; heuristic mode acceptable)
- All **D1-D3** (hook script + install + server.json discovery)
- All **F1-F6** (visual office; F6 ⚠ acceptable in half-block)
- All **G1-G3** (FSM)
- All **H1-H3** (assets)
- All **J1-J2** (seats from chairs)
- All **K1-K3** (atomic write)
- All **N1** (toolbar)
- All **O1-O2** (single-command install, launcher)
- All **Q1-Q2** (Linux + macOS)
- All **R1-R2** (frame budgets in T1-K + T3-foot/wez/mlterm)

**MVP dev-day budget: 80 dev-days (~10 weeks, 2 engineers).**

Breakdown: Phase 0 (8) + Phase 1 (16) + Phase 2 (14) + Phase 3 MVP-only slice (16, drops F7-F14 + H4-H13 Full items) + Phase 4 MVP-only slice (8, drops B5-B7) + Phase 7 minimum subset (10, drops Windows + N3/N6 polish) + Phase 8 unit + E2E only (8). Multi-client (Phase 6), Layout editor (Phase 5), Windows support deferred to v1.1.

---

## Roadmap (post-MVP)

- **Phase 5 — Layout editor** (Section I, all Full)
- **Phase 6 — Multi-client** (Section L, K4-K9)
- **Phase 7 (full)** — Windows + tmux/zellij polish, sound (M), notifications, settings modal (N2-N6)
- **Phase 8 — Full terminal compatibility matrix** (P5)
- **S1 — Agent-agnostic adapters** (Codex, OpenCode, Gemini, Cursor). `AgentRuntime` interface keeps this open.
- **S2 — Kanban-board wall**, drag-to-assign
- **S3 — Token health bars** — `agent.tokenUsage` event already in §10
- **S4 — 3D / VR future** — rendering tier is pluggable
- **Homebrew formula** — after v1.0 stabilizes
- **Auto-update with "apply" mode** — deliberately deferred; manual upgrade only

---

## Risk register (top 8)

| #   | Risk                                                                 | Severity | Mitigation                                                                                                        | Owner      |
| --- | -------------------------------------------------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------- | ---------- |
| 1   | Phase 0 introduces regression in dual-mode session detection         | High     | Fixture-based regression tests written before refactor; full Playwright e2e gate before merge                     | Engineer A |
| 2   | `wezterm-term 0.22` API churn during build                           | Medium   | Pin `=0.22.0`; vendor `tattoy-wezterm-term` as backstop                                                           | Engineer B |
| 3   | FSM parity drift between TS engine and Rust port                     | High     | Shared JSON fixtures in `tests/fixtures/`; both suites consume same data; `worldSeed` tick-for-tick parity test   | Engineer B |
| 4   | `--resume` failure paths leak agents into `agents.json` indefinitely | Medium   | Explicit drop-from-persistence on missing JSONL / stale JSONL / unknown session per §16 failure-paths table       | Engineer A |
| 5   | Terminal compatibility matrix CI cost/flakiness                      | Medium   | Docker pin tags; `claude-mock` for hermetic E2E; Windows runners only PR-trigger                                  | Engineer B |
| 6   | Asset blob channel chunk reassembly bugs                             | Medium   | EOF-bit on `tier` byte; fuzz tests for arbitrary split points                                                     | Engineer A |
| 7   | Multi-client conflict resolution leaves stale layout in some client  | Medium   | Writer-tag bootId match; 10 s dirty-edit suppression; `STALE_LAYOUT` error on conflicting save                    | Engineer A |
| 8   | Cooperative-with-extension mode causes duplicate hook delivery       | Medium   | Hook event dedup by `(sessionId, hook_event_name, timestamp)`; first-write-wins on `agent.toolStart` per `toolId` | Engineer A |

---

## Critical path

**Must serialize.**

- Phase 0 → Phase 1 (daemon cannot reuse modules until interface-injected)
- Phase 1 → Phase 4 (PTY hosting needs daemon RPC)
- Phase 1 → Phase 6 (multi-client needs daemon broadcast)
- Phase 3 (asset pipeline + FSM port) → Phase 5 (editor uses same `canPlaceFurniture`)
- Phase 7 (packaging) → Phase 8 (release)
- Within Phase 3: daemon asset pipeline (days 1-3) → client tier encoders (days 10-16) → snapshot tests (day 22)

**Can parallelize.**

- Phase 1 (daemon foundation) and Phase 2 (client foundation) — start the same day after Phase 0 ships. Both consume the locked §10 wire protocol.
- Phase 4 (PTY) and Phase 5 (editor) — concurrent after Phase 3's render tiers ship (~day 17 of Phase 3).
- Phase 6 (multi-client) and Phase 7 (packaging) — concurrent after Phase 5 lands.

**Bottlenecks.**

- Phase 3 day 4-7 (engine port) — single-engineer on B; consider pair-programming with A.
- Phase 8 day 7-10 (terminal matrix) — needs hardware/Docker; pre-stage during Phase 7.

---

### Document metrics

- **Total dev-day estimate for MVP:** 80 dev-days, ~10 weeks calendar (2 engineers).
- **Total dev-day estimate full plan:** 120 dev-days, ~15 weeks calendar.
