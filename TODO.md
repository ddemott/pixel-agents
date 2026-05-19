# TODO

Active items by priority. For background and big-picture plans, see the linked docs at the bottom.

## Now — Phase 2

- [ ] Phase 2 Rust TUI client — remaining: capability detection (DA1/Kitty/iTerm2 probes + 150ms timeout, Day 5-6), pre-app input queue (Day 7), event loop skeleton (Day 8), focus FSM (Day 9), Ratatui chrome (Day 10), agent.list + agent.spawn (Day 11-12), reconnect + supervisor handoff (Day 13-14).

## Recently done

- ✅ Phase 2 Day 3-4 — Streaming framing decoder + encoder. `client/src/daemon/framing.rs`: `FrameDecoder` (streaming `push`/`drain`, poisoned-on-error), all 4 frame types (`Ndjson`, `PtyOut`, `PtyIn`, `Asset`), `encode_ndjson<T>()` + `encode_pty_in()` public encoders, test-only `encode_pty_out()` + `encode_asset()`. 21 unit tests: roundtrip + byte-by-byte for every frame type, mixed-type sequencing, split-header/split-payload resumption, oversized-frame and unknown-tag errors, poison propagation. `connection.rs` updated: send path uses `encode_ndjson`; recv path uses `recv_one_frame` loop feeding `FrameDecoder`. Smoke-tested: `cargo run` against live daemon → `connected: daemon 0.0.1 boot=<bootId> session=<sessionId>`. 21/21 tests pass, zero warnings.
- ✅ Phase 2 Day 1-2 — Rust TUI client scaffold + UDS handshake. `client/` Cargo workspace (`pa-tui` bin): `ratatui 0.30`, `ratatui-crossterm 0.1`, `crossterm 0.29`, `tokio 1`, `serde`/`serde_json 1`, `bytes 1`, `vte 0.15`, `tachyonfx 0.25`, `arboard 3`, `directories 6`, `image 0.25`, `anyhow 1`. Module tree: `daemon/{mod,wire,discovery,connection}.rs`. `wire.rs` ports all TypeScript types (`Hello`+`ClientCapabilities`+`HelloAck`+`Req`+`Res`+`Evt`+`Fatal`+`Inbound`) with correct serde `tag="kind"` internally-tagged enum. `discovery.rs` reads `~/.pixel-agents/daemon.json` via `directories`. `connection.rs` connects via `tokio::net::UnixStream`, sends `Hello` as NDJSON frame (`[0x00][json][0x0a]`), parses `HelloAck` with bootId pinning. Smoke-tested: `cargo run` against live daemon → `connected: daemon 0.0.1 boot=<bootId> session=<sessionId>`. 273/273 daemon Vitest tests still pass.
- ✅ Daemon supervisor integration — `daemon/src/supervisor.ts` (`installSupervisor({ nodePath?, scriptPath?, platform? })` test-seam-friendly; generates systemd unit / launchd plist / Windows Scheduled Task XML for the current node + script path). `--install-supervisor` flag added to `server.ts`; early-exits before any daemon boot logic, prints config path + activation command, never auto-enables. Fixed pre-existing build bug: `dist/daemon/src/hooks/package.json {"type":"commonjs"}` now written by build step alongside `dist/src/package.json` so CJS hooks resolve correctly at runtime. Smoke-tested: `npm start -- --install-supervisor` writes `~/.config/systemd/user/pixel-agents.service` with correct `ExecStart`. 9 new Vitest cases (linux/darwin/win32 content + activate command + alreadyExisted flag + unsupported platform error; 273/273 total).
- ✅ Phase 2 JSONL polling port — `daemon/src/agents/jsonlPoller.ts` (`JsonlPoller` class: per-agent 500 ms `setInterval`, `start(agentId, sessionId, cwd, jsonlPath, seedOffset)` / `stop(agentId)` / `stopAll()`. Seeds `fileOffset` at 0 for fresh spawns, `stat.size` for revivals (skips replaying history). Inlines `readNewLines` logic (64 KB cap, `lineBuffer` for partial lines, `cancelWaitingTimer`/`cancelPermissionTimer` on new data). Delegates to `processTranscriptLine` from `src/transcriptParser.ts`. `markHookDelivered(agentId)` sets `agent.hookDelivered = true` to suppress heuristic timers when hooks are active. `DaemonHookBridge.setHookDeliveredCallback(cb)` fires on any hook event delivered to a known agent (PreToolUse, PostToolUse, Stop, Notification, SessionEnd); bridge wired in `server.ts` to call `jsonlPoller.markHookDelivered`. Poller added as optional field on `DispatchContext` and `ReviveContext`. Wired: `agent.spawn` starts at offset 0; `agent.close` + spawn `onExit` stop. Revival: poller starts after 3 s health check passes at `jStat.size`; revival `onExit` stops. `stopAll()` called in daemon shutdown. 9 new Vitest cases using `vi.useFakeTimers()` + real `fs.appendFileSync` to drive poll ticks (264/264 total). Build clean.
- ✅ Phase 1 Day 15-16 — `claude --resume` revival on daemon restart. `daemon/src/agents/resume.ts` (`reviveAgentsOnBoot` iterates `agents.json` per-cwd: JSONL liveness gate (exists + mtime <30d), `claude --resume <sessionId>` via PtyHost, 3s health check via Promise.race). Seven failure paths: JSONL missing/stale → drop entry + log; exit 127 (claude_missing) → emit `agent.spawnFailed { reason: "claude_missing" }`, keep entry; exit 2 + "session format version mismatch" → `agent.spawnFailed { reason: "claude_upgraded" }`, keep entry; other early exit → drop; hangs → keep PTY alive. Successful revival emits `agent.created { isResumed: true }` + refreshes `lastSeenAt` in agents.json. `classifyExit()` helper also wired into `agent.spawn`'s `onExit` handler so `agent.exited` events ship `reason: "user" | "crash" | "claude_missing"`. Clean user-exit (`reason: "user"`) now removes the agent from persistence so daemon restart doesn't attempt `--resume` for closed sessions. Revival fires in background after daemon socket is open so clients connect immediately. `reviveAgentsOnBoot` call in `server.ts` after full context setup. 10 new Vitest cases using `vi.resetModules()` + dynamic imports for path isolation (255/255 total). Build clean.

## Technical debt surfaced during Phase 0-1

- [ ] `DaemonHookBridge` drops `SubagentStart` / `SubagentStop` hook events (default debug-log
      branch). Need `agent.subagentStart` / `agent.subagentEnd` topics so clients can render
      the parent character's Task subtask state. Surfaced during Day 13-14.
- [ ] `PtyHost.onData` calls `sink.broadcastPty(...)` unconditionally — never consults the
      Day 9-10 backpressure flag. `BroadcastSink.register` exposes `onPause`/`onResume`
      callbacks for exactly this. Wire the PTY pump to pause `pty.read()` when a connection
      backs up so we don't OOM on a wedged client.
- [ ] `src/configPersistence.ts` (VS Code extension) writes `{ externalAssetDirectories }`
      only — strips the daemon's `logLevel` field on next write. Move to a per-field patch
      that preserves unknown keys, or have the extension read+merge before writing.
- [ ] Hook script discovery chain (`daemon/src/hooks/providers/hook/claude/hooks/claudeHookSrc.ts`)
      still falls through to `server.json` after `daemon.json` lacks a `hookPort`. Now that
      the daemon owns the hook server, that branch is only reachable when the extension is
      running without a daemon. Either decide to deprecate the extension-hosted server
      (Phase 6) or leave the branch + add a comment that it's transitional.
- [ ] `agent.spawn` reports synchronous spawn failures via `err('spawn_failed', …)`, but
      node-pty's ENOENT for a missing `claude` binary lands asynchronously through `onExit`
      with exit code 127 — currently broadcast as a generic `agent.exited`. `agent.exited`
      now carries `reason: "claude_missing"` but the client-facing `agent.spawnFailed` toast
      is only emitted by `reviveAgentsOnBoot`, not by live `agent.spawn`. Add the same
      classification to `agent.spawn`'s onExit so clients can toast on fresh spawns too.
- [ ] `agent.spawn` persists `palette: 0, hueShift: 0` for every new agent. Port the
      extension's `pickDiversePalette()` (counts current characters, picks least-used; first
      6 get unique skin, beyond that repeats with random hue rotation).
- [ ] `daemon/src/hooks/eventHandler.ts:1` — `TODO(Standalone version)` comment calls for moving
      `timerManager` + `types` into `daemon/src/` to eliminate cross-package imports from
      `src/` via the widened `rootDir`. No urgency while the shared tsconfig include works,
      but correct long-term once Phase 3+ lands enough daemon-only infra.
- [ ] `daemon/src/hooks/package.json` CJS-scope override is a workaround for the extension's
      CJS scope. Long-term: either make the extension source ESM (esbuild still emits CJS) or
      split the hooks subtree to its own package. Works today; unusual.
- [ ] Daemon boot logic has no unit tests — only manually smoke-tested. Add once Day 5+ lands
      enough infrastructure to make the test meaningful.
- [ ] E2E suite covers exactly one scenario (clicking + Agent and seeing the JSONL appear).
      More scenarios would be high-value insurance against regressions in later phases.

## Design open questions (not blocking)

From `docs/tui-architecture.md` §23 — kept around because we'll need telemetry to answer them:

- Snapshot vs delta tick rate balance once clients are live.
- Sixel throughput on Windows Terminal (target 20 fps; currently ⚠).
- ConPTY edge cases on Windows with `claude`'s output sequences.
- HSBC color quantization in the T5/T6 tiers — fidelity vs memory trade-off (current
  choice is fidelity, store full HSBC and quantize at draw).

## Source-of-truth docs

- `docs/tui-implementation-plan.md` — phased build plan with Progress block
- `docs/tui-parity-checklist.md` — feature parity tracking (~100 items)
- `docs/tui-architecture.md` — frozen design reference
- `CLAUDE.md` — compressed file layout + key patterns

## Recently done

- ✅ Phase 1 Day 13-14 — Agent spawn + PTY hosting. `daemon/src/agents/ptyHost.ts` (`PtyHost` wrapper around node-pty; constructor injects `spawn` for tests, exposes `write` / `resize` / `kill`, fires `onData` / `onExit`). `daemon/src/agents/liveAgents.ts` (`LiveAgents` Map<id, LiveAgent> w/ allocate / reserve / add / get / bySession / remove). `BroadcastSink.broadcastPty(agentId, bytes)` ships raw bytes as 0x01 frames to subscribers of `agent:<id>` / `agent:*` / unfiltered; auto-splits payloads above the 1 MB framing cap. `agent.spawn` (sessionId UUID + alloc agentId + register session→agentId in hookBridge + spawn `claude --session-id <uuid>` + persist to AgentsRegistry + emit `agent.created` + return `{id, sessionId}`), `agent.close` (SIGTERM + SIGKILL escalation 2 s + depersist), `pty.input` (base64 → write), `pty.resize` (positive cols/rows). `DispatchContext` gains `liveAgents`, `hookBridge`, `logger`. Shutdown kills all live PTYs. `node-pty@^1.2.0-beta.13` dep. 12 new Vitest cases including real `/bin/cat` PTY round-trip (245/245 total). Live smoke: PIXEL_AGENTS_HOME=/tmp/pa-smoke2 daemon boots, publishes daemon.json, accepts SIGTERM cleanly.
- ✅ Phase 1 Day 12 — Hook integration test: HTTP → bridge → sink → mock client. New `daemon/src/hookHost/{server,bridge}.ts`: minimal 127.0.0.1 hook HTTP server (port 0, Bearer auth = daemon's UDS token, JSON body, `/api/hooks/:providerId`) + `DaemonHookBridge` (sessionId→agentId map, per-agent toolId stack, normalizes SessionStart/PreToolUse/PostToolUse/Stop/UserPromptSubmit/PermissionRequest/Notification to `agent.created`/`agent.toolStart`/`agent.toolDone`/`agent.statusChanged`/`agent.exited` topics). `daemon.json` now publishes `hookPort`. `PIXEL_AGENTS_HOME` env override in `paths.ts` so tests isolate from real daemon. `daemon/__tests__/hookHost/integration.test.ts` boots real BroadcastSink + bridge + HTTP server + UDS, drives end-to-end via POSTs + UDS subscribe. 3 new Vitest cases (232/232 total). Live smoke: `PIXEL_AGENTS_HOME=/tmp/pa-smoke node server.js --foreground` writes daemon.json with hookPort, accepts auth'd POSTs, `ok` reply.
- ✅ Phase 1 Day 11 — NDJSON logging. `daemon/src/logging/{logger,retention}.ts`: file-per-UTC-day at `~/.pixel-agents/logs/daemon-YYYY-MM-DD.log` (sync `openSync('a')` + `writeSync`, 0o600), pinned `{ts, level, module, agentId?, ..., msg}` key order, `setLevel()` for runtime updates, optional stderr mirror for `--foreground`. `logLevel` field added to `config/persistence.ts` (default `info`, watcher updates logger on external write). Retention sweep gzips `*.log` >7d → `*.log.gz`, deletes `*.log[.gz]` >30d; runs at boot + every 24 h (unref'd interval). `BroadcastSink.setLogger()` injection replaces its `console.error`. 13 new Vitest cases (229/229 total). Live smoke: boot/shutdown round-trip writes valid NDJSON.
- ✅ Phase 1 Day 9-10 — daemon-side `AgentEventSink` bus + per-agent scope + backpressure. `BroadcastSink.emitTo(agentId, event)` filters by per-conn `agent:<id>` / `agent:*` subscriptions (no `agent:` filter = implicit subscribe-all). Per-conn high-water-mark backpressure: `sock.write()` false flips subscriber to paused, frames spill into bounded `SUBSCRIBER_QUEUE_MAX = 256` ring (oldest dropped on overflow, `droppedFrames` counter for diagnostics); `'drain'` flushes + fires `onResume`. `register(sock, subs, { onPause, onResume })` exposes the hooks PTY pumps will gate on (Day 13-14).
- ✅ Phase 1 Day 7-8 — RPC command catalog: `daemon/src/rpc/dispatch.ts` (MethodRegistry, ConnectionScope, DispatchContext, `ok` / `err` helpers) + `daemon/src/rpc/methods/{layout,settings,subscribe,control,agents,index}.ts`. Implemented: `layout.get/save/import/export` (`save` debounced + broadcasts `layout.changed`), `settings.get/set`, `subscribe` (topic filter persisted on per-conn `ConnectionScope.subscriptions`), `daemon.shutdown`, `agent.list` (reads from `AgentsRegistry`). Gated as `not_yet_supported`: `agent.spawn/close/focus/reassignSeat/adopt`, `pty.input/resize/resync`, `assets.list/requestBlob/addDir/removeDir`, `hooks.toggle`, `layout.setDefault`. `BroadcastSink` extended w/ per-conn subscription filtering (empty = all, `["*"]` = wildcard). 21 new Vitest cases (209/209 total). Live RPC smoke: client successfully invokes `settings.get`, `layout.save` (sees broadcast), and gets `not_yet_supported` for `agent.spawn`.
- ✅ Phase 1 Day 6 — Persistence ports + writer-tag (arch §16): `daemon/src/persistence/{writerTag,watcher}.ts` (atomic tmp+rename + `_writer { processId, bootId }` tagging; `fs.watch` + 2 s polling backup; own-write filtered by bootId match). `daemon/src/layout/persistence.ts` (read/write/watch + `LayoutSaveDebouncer` 500 ms coalesce). `daemon/src/config/persistence.ts` (replaces old `daemon/src/config.ts`). `daemon/src/agents/registry.ts` (typed per-cwd `agents.json`: `{version:1, agents:{[cwd]:PersistedAgent[]}, _writer}`). `FileStateStore` repointed at `daemon-state.json`. Server boot loads layout + config, starts watchers, broadcasts `layout.changed` / `settings.updated` evts on external edits. 18 new Vitest cases (188/188 total). Live smoke: writing an external-tagged `layout.json` immediately ships a `layout.changed` evt to the connected client.
- ✅ Phase 1 Day 5 — Phase-0 modules wired into daemon: cross-package tsconfig include for `src/{messageSender,terminalRegistry,agentRuntime,types,timerManager,transcriptParser}.ts`, `BroadcastSink` (`AgentEventSink` impl fanning out over UDS w/ per-topic monotonic seq), `DaemonRuntime` (`AgentRuntime` from boot cwd), `FileStateStore` (`AgentStateStore` backed by `agents.json` w/ atomic tmp+rename). `onAuthenticated` callback on RPC connection registers sock with sink. Build emits a `dist/src/package.json {"type":"commonjs"}` scope override so Node 22 ESM can interop with the Phase-0 CJS modules. 9 new Vitest cases (170/170 total).
- ✅ Phase 1 Day 3-4 — RPC framing on UDS: channel mux (`framing.ts`), `wire.ts` types, `connection.ts` handler with token auth + `helloAck` w/ inline (stub) `WorldSnapshot`. 21 Vitest cases.
- ✅ Phase 1 Day 2 — port `server/` → `daemon/src/hooks/` + discovery chain + esbuild fix (`47c2288`, `b7ef2f3`, `08f5064`)
- ✅ Phase 1 Day 1 — daemon scaffold + `config.json` read (`ab77a32`, `764da25`)
- ✅ Phase 0 — MessageSender / TerminalRegistry / AgentRuntime decoupling (`3d36a3c`, `a6984c4`)

## Historical — do not edit

These are frozen snapshots of past states (rewriting them rewrites history):

- `docs/critique-r1.md`, `docs/critique-r2.md` — design-loop critiques
- `docs/changes-r1-to-r2.md`, `docs/changes-r2-to-r3.md` — design-loop deltas
