# Pixel Agents TUI Architecture (v3 — Critique-R2 Response)

> **Revision:** v3. Supersedes v2 by surgical edit, not rewrite — the architecture is unchanged. Every Critical, Major, and Minor item from `docs/critique-r2.md` (NEW-CRIT-1, NEW-MAJ-1..6, NEW-MIN-1..7) is addressed in-place; each fix is annotated with its critique ID. The full v1→v2 deltas live at `docs/changes-r1-to-r2.md`; v2→v3 deltas at `docs/changes-r2-to-r3.md`.
>
> **Status (design):** every MVP item in `docs/tui-parity-checklist.md` is **✓** in §22; the only ⚠ items are Full-tier where the limitation is explicit and product-acceptable.
>
> **Status (build):** this document was authored before any code shipped and references `server/src/...` paths from the pre-Phase-0 layout. For the current build state, file layout, and which phases have shipped, see `docs/tui-implementation-plan.md`. The `server/` directory referenced throughout this doc is now `daemon/src/hooks/` post Phase 1 Day 2.

---

## 1. Executive Summary

Pixel Agents-TUI is a daemon + thin-client architecture. A long-lived **TypeScript daemon** (an evolution of the existing `server/` tree, plus refactored slices of the current VS Code extension's `src/`) owns agent registry, PTY lifecycle, JSONL watching, the Claude hook server, persistence, and the asset model. One or more **Rust TUI clients** (Ratatui 0.30 modular workspace + `ratatui-crossterm` 0.1 + crossterm 0.29) connect over a Unix domain socket (named pipe on Windows) and render the animated pixel-art office plus the focused agent's terminal.

The four headline decisions that shape everything else:

1. **PTY parser strategy (Addresses CRIT-1).** We use **`wezterm-term` 0.22+** as the headless terminal emulator, with a **raw-byte tap upstream of the parser** to preserve image escapes (Kitty, iTerm2, Sixel) and to drive the in-terminal scrollback verbatim. `wezterm-term` exposes the model via `advance_bytes()` and ships first-class Sixel + iTerm2 image cells; it is actively maintained and explicitly designed to be embedded (no GUI dependencies). `alacritty_terminal` is rejected for this role — its 0.25 API surface still warns that it is "primarily for use within Alacritty" and image escapes are silently swallowed.

2. **Game-loop authority is client-side (Addresses CRIT-3).** The daemon broadcasts **events + a canonical world model** (assets, layout, agent registry, deterministic spawn seed); each client runs its **own** `OfficeState` FSM at 60 Hz from the event stream. Since the existing engine code is pure and seed-driven, this is a straight port of `webview-ui/src/office/engine/` into Rust with no new logic. F6 pixel-perfect is preserved because animation phase is computed per-client.

3. **Phase 0 (Addresses CRIT-4): land the `MessageSender` interface in the existing repo first.** Verified `grep -c "vscode" src/{fileWatcher,agentManager,transcriptParser,timerManager}.ts` on 2026-05-19 returns **41 lines** (19 + 15 + 3 + 4 — see §11 for the per-file breakdown and the v1↔v2↔v3 counting-methodology reconciliation, Addresses NEW-MAJ-6) and these files pass `webview: vscode.Webview | undefined` through nearly every signature. We extract a `MessageSender` (a.k.a. `AgentEventSink`) interface, refactor in-place behind it, ship through CI on the current VS Code extension, _then_ begin the daemon port. The §11 table now distinguishes "behind-MessageSender (verbatim)" from "still couples to vscode (rewrite)".

4. **Restoration uses both an OS supervisor and `claude --resume` (Addresses CRIT-2).** The daemon is supervised by a per-user systemd unit / launchd plist / Windows Scheduled Task (we ship all three). On daemon boot, persisted agents from `~/.pixel-agents/agents.json` are revived by `claude --resume <sessionId>` per entry, with a JSONL liveness gate. Both safeguards are required: the supervisor keeps the daemon alive across host-process crashes; `--resume` keeps each Claude session alive across daemon restarts even though child PTYs die with their parent.

Supporting decisions, summarized:

- **Daemon language: TypeScript on Node.js 22 LTS.** Reuses the entire `server/` tree and the four refactored extension modules.
- **Client language: Rust 1.79+ with Ratatui 0.30 (modular workspace) + `ratatui-crossterm` 0.1 + crossterm 0.29.** Single static binary, deterministic latency.
- **Wire protocol: NDJSON for control + a length-prefixed binary multiplex for PTY (Addresses MAJ-2).** PTY data is no longer base64-in-JSON. Frame format `0x01 streamId:u32 len:u32 bytes`.
- **Capability detection happens before raw mode where possible; a pre-app input queue absorbs spurious bytes; `PIXEL_AGENTS_TIER` env var force-overrides (Addresses MAJ-3).**
- **server.json is owned cooperatively (Addresses CRIT-5).** Extension + daemon both consult it. Whichever boots first wins; the other adopts. Hook script tries `daemon.json` then falls back to `server.json` (env override `PIXEL_AGENTS_HOOK_URL` short-circuits both).
- **Multi-client** is first-class. Each client has its own viewport, zoom, focus, hover, selection, undo stack. Layout edits broadcast.
- **Distribution**: npm package `pixel-agents` for the daemon; pre-built Rust client binary downloaded per-platform from GitHub Releases (sha-pinned, opt-in updates only — see §18, Addresses MAJ-10).

---

## 2. Goals & Non-Goals

### Goals

- Every MVP item in `tui-parity-checklist.md` is **✓**; every Full item is **✓** or **⚠** with explicit accepted limitations.
- 60 fps in image tiers (Kitty, iTerm2). 30 fps in Sixel; 60 fps trivially in half-block tiers.
- Visually faithful port of the VS Code webview office. Characters animate identically; layout editor has equivalent power.
- Faithful Claude session lifecycle (spawn, `/clear`, `/resume`, sub-agents, teammates, permission detection).
- Cross-platform: Linux, macOS, Windows. Works under tmux/zellij/screen with documented limitations.
- Daemon survives client crashes; clients reconnect and re-render. Agent processes survive **both** client and daemon restarts via `--resume` (see §17).

### Non-Goals (MVP)

- Mouse parity with native GUIs. SGR mouse where available; keyboard fallback otherwise.
- True alpha blending. Pixel art is alpha 0 or 1; semi-transparency is approximated as opaque after foreground premultiply in fallback tiers.
- Embedded audio synthesis. We shell out (§15).
- Native (3D / VR) renderer (S4) — only ensured non-blocked.
- Replacing the VS Code extension. Both targets share the refactored `server/` + four-file core going forward.

---

## 3. High-Level Architecture

```
                                     ~/.pixel-agents/
                                     ├── daemon.json    (daemon owns) | server.json (extension owns)
                                     ├── socket         UDS or \\.\pipe\…
                                     ├── layout.json    atomic write + writer-tag
                                     ├── config.json    atomic write
                                     ├── agents.json    per-cwd persisted agents
                                     ├── hooks/claude-hook.js   bundled CJS
                                     └── logs/daemon-YYYYMMDD.log

         ┌────────────────────────────────────────────────────────────────────┐
         │                pixel-agents-daemon  (Node.js 22 LTS)                │
         │                                                                    │
         │   ┌──────────┐  ┌────────────┐  ┌──────────────────────┐           │
         │   │ AgentMgr │◄─┤ JSONL      │  │ HookHTTPServer       │◄──── claude-hook.js
         │   │ (PTYs)   │  │ Watcher    │  │  127.0.0.1:auto      │      (per Claude CLI session)
         │   └────┬─────┘  └────┬───────┘  └──────────┬───────────┘           │
         │        │             │                     │                       │
         │        ▼             ▼                     ▼                       │
         │   ┌────────────────────────────────────────────────────┐           │
         │   │                 AgentEventSink (bus)               │           │
         │   └──────┬─────────────────────────────────────────────┘           │
         │          ▼                                                          │
         │   ┌─────────────────────────┐    ┌──────────────────────────┐     │
         │   │   RPC + Stream Hub      │    │ LayoutPersistence         │     │
         │   │   - NDJSON control      │    │ ConfigPersistence         │     │
         │   │   - Binary PTY mux      │    │ AssetCatalog              │     │
         │   │   - Multi-client        │    │ Supervisor (systemd/etc.) │     │
         │   └────┬───────┬───────┬────┘    └──────────────────────────┘     │
         └────────┼───────┼───────┼─────────────────────────────────────────-─┘
                  │       │       │
            ┌─────▼──┐ ┌──▼───┐ ┌─▼────────┐
            │Client A│ │  B   │ │  C ...   │   (Rust TUI clients)
            │Kitty   │ │tmux  │ │WinTerm   │
            │protocol│ │+Sixel│ │+Sixel    │
            └────────┘ └──────┘ └──────────┘
                  ▲ event stream + world model
                  │ each client runs its own OfficeState FSM
                  │ keyed by shared worldSeed (in `hello` response)

         ~/.claude/projects/<hash>/<sess>.jsonl     (daemon watches)
         daemon hosts PTY → bytes pumped over binary mux → client `wezterm-term` grid
```

The daemon owns _all canonical state_: agents, layout, assets, hook events. Clients are **deterministic FSM replicas** plus pure views. They keep per-viewport ephemera (zoom, pan, hover, selection, editor tool, dirty buffer, per-client undo stack). Layout edits travel client → daemon → all-other-clients → persisted file (with writer-tag — Addresses MAJ-11).

---

## 4. Process Model

### Spawning

- `pixel-agents` (the launcher binary, written in Rust and shipped alongside the client) reads **both** `~/.pixel-agents/daemon.json` and `~/.pixel-agents/server.json` (Addresses CRIT-5). Resolution rule:
  1. If `daemon.json` exists and its `PID` is alive and `bootId` matches a successful `GET /api/health` reply within 250 ms → attach client to that daemon.
  2. Else if `server.json` exists and points at a live VS Code extension server → daemon mode is "extension-cooperative": launcher starts a daemon process that does **not** install hooks (the extension already has) and that publishes only `daemon.json`. Both processes coexist; clients connect to `daemon.json`; hook events flow through whichever server has registered the agent first.
  3. Else launcher forks a detached `pixel-agents --daemon`. The daemon writes `daemon.json` atomically and binds the UDS at `~/.pixel-agents/socket` (Windows: `\\.\pipe\pixel-agents-<sha1(user@host)>`).
- Cold-start retry: launcher waits up to **3 s** for socket readiness (was 2 s in v1; raised to absorb supervisor handoff). Two probes at 250 ms / 1 s; if both fail, the launcher attempts a one-shot daemon respawn before giving up.
- On success the launcher `exec`s `pixel-agents-tui`, passing the socket path via `PIXEL_AGENTS_SOCKET` and the auth token via `PIXEL_AGENTS_TOKEN`.

### bootId UUID rotation (Addresses CRIT-5)

Each daemon process generates a new `bootId` UUIDv4 at startup and writes it into `daemon.json`. `bootId` rotation policy:

- Rotated on every daemon process start (including supervisor-triggered restarts).
- **Not** rotated when only the auth token changes (e.g. a user manually edits the token).
- Clients pin to the `bootId` returned in the `hello` response. If a subsequent socket read sees a different `bootId` (because the daemon restarted under them), they treat the connection as dead, drop subscriptions, and reconnect via the launcher path. This eliminates "stale token / surviving connection" classes of bug.

### Corruption recovery (Addresses CRIT-5)

If `daemon.json` is unreadable (truncated, partial-write crash, non-JSON), the launcher logs a warning, **unlinks** the file, and proceeds as if it didn't exist. The daemon's writer is atomic (`fs.writeFile` of `daemon.json.tmp` + `fs.rename`); a partial-write corruption can therefore only occur if a non-pixel-agents process wrote there.

### Ownership

- **Daemon owns**: `node-pty` PTY handles, hook HTTP server, JSONL pollers, layout file watcher, agent registry, hook event buffer, layout file, config file, agents file, asset catalog, sound cascade.
- **Client owns**: viewport state, capability profile, render cache, input loop, scrollback ring per agent (mirrored deltas), `wezterm-term::Terminal` per agent, **its own OfficeState FSM driven by event stream + seed** (Addresses CRIT-3).

### Lifecycle table

| Event                          | Behavior                                                                                                                                                                                                                                                             |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Client crashes                 | Daemon detects EPIPE on socket; drops client subscriptions; PTYs continue. Daemon stays alive.                                                                                                                                                                       |
| Daemon crashes                 | Supervisor (systemd-user / launchd / Scheduled Task — see §17 + §4 below) restarts daemon. Clients see `read() = 0`; show "Reconnecting…" overlay; relaunch via launcher logic (which reattaches to the supervised-restarted daemon). Agent restoration follows §16. |
| PTY dies (claude exits)        | `node-pty` `onExit` → daemon emits `agent.exited`. Client triggers despawn matrix effect. Agent record removed from persistence.                                                                                                                                     |
| Orphaned daemon (no clients)   | Daemon stays alive for `IDLE_DAEMON_LIFETIME_MS` (default 60 min) then self-exits cleanly, allowing layout watcher to flush. Configurable; `0` = forever.                                                                                                            |
| Client sends `daemon.shutdown` | Daemon stops accepting new connections, sends `daemon.shuttingDown` to all clients, terminates PTYs (`SIGTERM` then `SIGKILL` after 2 s), unlinks `daemon.json`.                                                                                                     |
| Supervisor sends SIGTERM       | Daemon flushes layout + config, kills PTYs cleanly, exits 0. Supervisor will not restart on a clean exit.                                                                                                                                                            |

### Supervisor strategy (Addresses CRIT-2)

We ship three supervisor configurations and install one based on the host OS. Postinstall (`npm install -g pixel-agents`) places them and prints the activation command — never auto-enables (Addresses MAJ-10).

**Linux** — `~/.config/systemd/user/pixel-agents.service`:

```ini
[Unit]
Description=Pixel Agents daemon (per-user)
After=default.target

[Service]
Type=simple
ExecStart=%h/.local/share/pixel-agents/bin/pixel-agents --daemon --foreground
Restart=on-failure
RestartSec=2s
SuccessExitStatus=0
KillMode=mixed
KillSignal=SIGTERM
TimeoutStopSec=10
Environment=NODE_ENV=production

[Install]
WantedBy=default.target
```

Activate: `systemctl --user enable --now pixel-agents.service`.

**macOS** — `~/Library/LaunchAgents/com.pixelagents.daemon.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.pixelagents.daemon</string>
  <key>ProgramArguments</key><array>
    <string>/usr/local/bin/pixel-agents</string>
    <string>--daemon</string>
    <string>--foreground</string>
  </array>
  <key>KeepAlive</key>
  <dict><key>SuccessfulExit</key><false/><key>Crashed</key><true/></dict>
  <key>ThrottleInterval</key><integer>2</integer>
  <key>StandardOutPath</key><string>/tmp/pixel-agents.out</string>
  <key>StandardErrorPath</key><string>/tmp/pixel-agents.err</string>
</dict>
</plist>
```

Activate: `launchctl load -w ~/Library/LaunchAgents/com.pixelagents.daemon.plist`.

**Windows** — Scheduled Task XML (`pixel-agents.xml`), loaded with:

```
schtasks /Create /TN "PixelAgents" /XML pixel-agents.xml
```

```xml
<?xml version="1.0" encoding="UTF-16"?>
<Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <LogonTrigger><Enabled>true</Enabled></LogonTrigger>
  </Triggers>
  <Actions>
    <Exec>
      <Command>%LOCALAPPDATA%\Programs\pixel-agents\pixel-agents.exe</Command>
      <Arguments>--daemon --foreground</Arguments>
    </Exec>
  </Actions>
  <Settings>
    <RestartOnFailure>
      <Interval>PT5S</Interval>
      <Count>3</Count>
    </RestartOnFailure>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
  </Settings>
</Task>
```

**All three supervisor configurations agree on restart policy (Addresses NEW-MIN-4)**: only crashes / nonzero exits trigger restart; clean exits stay down (user-initiated `pixel-agents-daemon stop`, or `daemon.shutdown` RPC). This is encoded as `Restart=on-failure` + `SuccessExitStatus=0` on systemd; `KeepAlive.SuccessfulExit=false` + `KeepAlive.Crashed=true` on launchd; and `RestartOnFailure` (without an "on success" sibling) on Windows Scheduled Task. The lifecycle-table row "Supervisor sends SIGTERM → exits 0; supervisor will not restart" reflects the same contract from the daemon's side.

**Graceful-shutdown semantics so PTYs survive transient restarts.** The daemon process _dies_ with its PTY children — there is no way around this on POSIX. The compensating mechanism is `claude --resume` (§16). On daemon graceful shutdown we explicitly do **not** persist scrollback (it is a runtime cache); persisted state is JSONL files (already on disk under `~/.claude/projects/`) plus our `agents.json`. On startup the new daemon respawns each persisted agent via `claude --resume <sessionId>` and rebinds via hooks (§16). Transient (sub-second) restart cycles are still **disruptive** to the user — they see Claude restart — but session continuity is preserved.

---

## 5. Daemon Design

### Language: TypeScript / Node.js 22 LTS

Same rationale as v1: the entire `server/`, plus `transcriptParser.ts`, `fileWatcher.ts`, `timerManager.ts`, `agentManager.ts`, `layoutPersistence.ts`, `configPersistence.ts`, `assetLoader.ts`, and `hookEventHandler.ts` are battle-tested. A Rust rewrite would risk parity bugs in the dual-mode session detection without compensating gain. Node 22 LTS has stable `node:fs/promises`, `WebSocket` global, structured clone, `worker_threads`, and `node:net` UDS.

### Module layout (`daemon/src/`)

```
daemon/src/
  server.ts                — boot, signal handling, supervisor-cooperative exit
  rpc/
    socket.ts              — UDS / named-pipe listener
    framing.ts             — NDJSON line + binary mux (§10)
    auth.ts                — token check (reuse server.json + daemon.json token)
    session.ts             — per-client subscription state
    catalog.ts             — RPC method table, JSON schema validation
  agents/
    agentManager.ts        — REFACTORED + ported from src/agentManager.ts (Phase 0)
    ptyHost.ts             — node-pty wrapper, byte stream pump, resize
    scrollback.ts          — bounded ring buffer per agent (256 KB default)
    registry.ts            — Map<agentId, AgentState> + persist to agents.json
    resume.ts              — `--resume` revival on daemon boot (§16)
  watching/
    jsonlWatcher.ts        — REFACTORED port of src/fileWatcher.ts (Phase 0)
    transcriptParser.ts    — REFACTORED port of src/transcriptParser.ts (Phase 0)
    timerManager.ts        — REFACTORED port of src/timerManager.ts (Phase 0)
  hooks/
    httpServer.ts          — REUSE server/src/server.ts (verbatim)
    eventHandler.ts        — REUSE server/src/hookEventHandler.ts (behind MessageSender)
    installer.ts           — REUSE server/src/providers/file/claudeHookInstaller.ts
  layout/
    persistence.ts         — REUSE src/layoutPersistence.ts (+ writer-tag)
    watcher.ts             — fs.watch + 2s polling; multi-client broadcast
    serializer.ts          — REUSE from webview-ui/src/office/layout/
  assets/
    loader.ts              — REUSE src/assetLoader.ts
    manifest.ts            — manifest.json parsing per furniture dir
    catalog.ts             — REUSE webview-ui/src/office/layout/furnitureCatalog.ts
    pipeline.ts            — NEW: pre-render sprites into per-tier representations,
                              lazy variants (§13 — Addresses MAJ-8)
  config/
    settings.ts            — REUSE src/configPersistence.ts
  audio/
    sound.ts               — cascade: pw-play→paplay→aplay→osascript→PowerShell
                              + notify-send/terminal-notifier/BurntToast (§15, MAJ-7)
  events/
    bus.ts                 — typed AgentEventSink (the MessageSender interface, §11)
  types.ts                 — shared interfaces (port src/types.ts)
  constants.ts             — REUSE server/src/constants.ts (single source)
```

### Authority split: who runs the game loop? (Addresses CRIT-3)

**Decision: clients own the FSM. Daemon broadcasts events + the canonical world model.**

What the daemon broadcasts:

- A **world model snapshot** delivered inline on the `HelloAck` response (`HelloAck.world`, Addresses NEW-MAJ-1), and re-emitted as a `world.snapshot` event on canonical state changes (layout edited, asset reload, agent created/removed, seat reassigned). The model contains layout, asset catalog ids, agent registry (id, palette, hueShift, seatId, current tool, etc.), and the `worldSeed` (a u32, generated once per daemon boot from `crypto.randomBytes(4)`).
- **Events** as they occur: `agent.created`, `agent.toolStart`, `agent.toolDone`, `agent.statusChanged`, `agent.toolPermission`, `agent.toolsClear`, `pty.data`, `layout.changed`, …

What each client computes locally:

- The **FSM tick** at 60 Hz. Characters walk, type, read, wander, return to seat, spawn matrix effect — all of this runs from the existing pure code in `webview-ui/src/office/engine/`, ported to Rust 1:1.
- The **wander RNG** is seeded with `worldSeed XOR agentId`. Because seed + same event stream = same random walks, multiple clients show identical wander behavior. Verified by snapshot test.
- The **matrix spawn/despawn effect timestamp** is the event arrival time (`now`). Effect duration is fixed (0.3 s). Two clients receiving an event 5 ms apart will diverge by 5 ms of effect phase — invisible.

#### Worked example: one agent walks to seat, types Write, returns to wander

| Wall clock  | Event on bus                                                      | Each client's local state change                                                                                                                                |
| ----------- | ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| t=0         | `agent.created { id:7, palette:2, hueShift:0, seatId:"chair-A" }` | Spawn character at random walkable tile near seat-A; FSM=`idle`; matrix-spawn effect begins, t=0 phase.                                                         |
| t=0.3s      | (no event)                                                        | Matrix-spawn ends; FSM continues.                                                                                                                               |
| t=1.0s      | `agent.toolStart { id:7, toolId:"call_abc", toolName:"Write" }`   | Lookup tool→action: Write ⇒ `type` animation, target=seat. FSM transitions to `pathfind`. BFS to seat-A using seed-derived tie-break.                           |
| t=1.0–3.2s  | (no event)                                                        | Character animates walk frames (walk1/walk2/walk3) cycling at `WALK_FRAME_HZ`. Position interpolated per-frame from pure FSM. **No frames sent over the wire.** |
| t=3.2s      | (no event)                                                        | FSM arrives at seat-A; sits (6px offset), starts `type` frames (type1/type2).                                                                                   |
| t=4.5s      | `agent.toolDone { id:7, toolId:"call_abc" }`                      | FSM transitions to `idle-at-seat`. Animation switches to walk2 standing pose.                                                                                   |
| t=4.5–14.5s | (no event)                                                        | After `wanderDelayMs` (10 s default) FSM picks a random walkable target via seeded RNG. Pathfinds, walks (walk frames).                                         |
| t=14.5–20s  | (no event)                                                        | Wander. After `wanderLimit` moves, returns to seat.                                                                                                             |
| t=20s       | (no event)                                                        | Character sits at seat-A. FSM=`idle-at-seat`.                                                                                                                   |

The daemon emitted exactly **3 events** for a 20-second sequence with hundreds of animation frames. Bandwidth at steady state with 20 agents: ≈100 B/event × ~0.5 events/sec/agent = 1 KB/s control. PTY data is separate (§10). Far below the v1's hand-waved "60 KB/s ceiling".

#### What about WorldSnapshot then?

The **initial** WorldSnapshot is delivered inline on the `HelloAck` response (`HelloAck.world`), not as a separate event — so the very first byte the client sees after a successful handshake already contains the full world model (Addresses NEW-MAJ-1). After that, we emit a `world.snapshot` event only on _canonical_ state changes (layout edit, agent registry change, asset hot-reload). It carries the world model — **not** character positions. Format spelled out in §10's `WorldSnapshot` schema. Steady-state snapshot rate: < 1/min.

#### Edge cases handled by client-side FSM

- **Late-joining client.** On `hello`, daemon replies with the current world model + a list of currently-active events to replay (agent.toolStart for each in-flight tool, etc.). The new client computes "the character should be sitting at seat-A typing Write" from event replay; if a wander began on another client 4 s ago and the new client misses that, the new client's wander starts fresh at its arrival time. Wander divergence is invisible (the user observes the new client's animation in isolation; multi-monitor users will see ≤ a few seconds of phase drift, which is acceptable).
- **Synchronization-critical effects** (matrix spawn/despawn) carry an absolute `t0` in the event payload, computed by the daemon. Clients lock effect phase to `now() - t0` so two clients see the effect in lockstep.
- **Per-frame "tool active" pulse animation** is pure local clock, not synced — same as today's webview.

### Public RPC + Event API (catalog at §10)

Commands (client → daemon) are request/response with a `reqId`. Events (daemon → client) are unsolicited broadcasts to subscribed topics. PTY data uses the binary mux (§10) not events.

### Data model (high-level)

```ts
// daemon/src/types.ts
export interface AgentState {
  id: number;
  sessionId: string;
  pty: { pid: number; rows: number; cols: number; alive: boolean };
  projectDir: string;
  cwd: string;
  jsonlFile: string;
  fileOffset: number;
  lineBuffer: string;
  activeToolIds: Set<string>;
  activeToolStatuses: Map<string, string>;
  activeToolNames: Map<string, string>;
  activeSubagentToolIds: Map<string, Set<string>>;
  activeSubagentToolNames: Map<string, Map<string, string>>;
  backgroundAgentToolIds: Set<string>;
  isWaiting: boolean;
  permissionSent: boolean;
  hadToolsInTurn: boolean;
  hookDelivered: boolean;
  pendingClear: boolean;
  inputTokens: number;
  outputTokens: number;
  palette: 0 | 1 | 2 | 3 | 4 | 5;
  hueShift: number; // 0–359
  seatId?: string;
  scrollback: RingBuffer; // bounded, 256 KB
}
```

The runtime mutable visualization (character x/y/dir/state) **lives on each client**, not in `AgentState`. The daemon only knows that an agent is associated with a seat and what tools are active.

### Map / Set wire serialization (Addresses MAJ-1)

`AgentState` is never serialized directly. The wire types are TS interfaces that use plain arrays/objects (see §10). Where the runtime uses `Set<string>` we emit `string[]`; where the runtime uses `Map<K,V>` we emit `[K, V][]` (tuple array). Schemas are codegen'd to JSON Schema and shipped to the Rust client via `schemars` macros.

### Concurrency

Single Node event loop, no threads. Each PTY pump uses standard backpressure (`pty.pause()` / `pty.resume()` on socket high-water mark). JSONL pollers are `setInterval`. The hook HTTP server is blocking on JSON parse only for ≤64 KB bodies. We do not use worker threads — the workload is I/O-bound.

---

## 6. TUI Client Design

### Language: Rust 1.79+

Justifications:

- Static binary, no runtime to install; trivial cross-compile (we ship glibc 2.31 + musl variants for Linux).
- Crossterm/Ratatui ecosystem is mature in May 2026. Ratatui shipped its 0.30 modular workspace release (separating `ratatui-core`, `ratatui-crossterm`, and other backends into their own crates), and `tachyonfx` 0.24 tracks that release with first-class compatibility (Addresses NEW-CRIT-1).
- Per-cell rendering of thousands of cells at 60 fps; GC pauses unacceptable.
- `wezterm-term` is the only mature embeddable terminal emulator with first-class image-cell support (Sixel/iTerm2 inline). The Kitty graphics protocol is handled by raw-byte passthrough (§9 — Addresses CRIT-1).

Alternatives considered (and why not):

- **Go + tcell + tview** — tview is heavyweight; tcell lacks a Kitty graphics writer as of 2026.
- **Node Ink** — React-style TUI; reuses TS skills but performance does not hold for animated full-screen renders at 60 fps.
- **Zig** — no production-grade TUI ecosystem.

### Library choices (May 2026 versions, verified)

| Crate                  | Version | Purpose                                                                                  |
| ---------------------- | ------- | ---------------------------------------------------------------------------------------- |
| `ratatui`              | 0.30.x  | Immediate-mode TUI rendering (modular workspace umbrella crate)                          |
| `ratatui-core`         | 0.1.x   | Core traits/types (transitive via `ratatui`; pinned for widget interop)                  |
| `ratatui-crossterm`    | 0.1.x   | Crossterm backend, now a separate crate as of the 0.30 split                             |
| `crossterm`            | 0.29.x  | Terminal I/O, raw mode, mouse, resize events                                             |
| `wezterm-term`         | 0.22.x  | Headless terminal emulator (PTY → grid) — **see §9**                                     |
| `image`                | 0.25.x  | PNG decode for client-side fallback assets                                               |
| `tokio`                | 1.47.x  | Async runtime for socket + input                                                         |
| `serde` / `serde_json` | 1.x     | NDJSON wire protocol                                                                     |
| `bytes`                | 1.x     | Zero-copy buffers for the binary PTY mux                                                 |
| `vte`                  | 0.13.x  | Pre-app input drain parser (consumes capability-probe replies — §7)                      |
| `tachyonfx`            | 0.24.x  | Optional effect overlays (matrix sweep); 0.24 is the first release tracking ratatui 0.30 |
| `arboard`              | 3.x     | Clipboard for copy-paste from scrollback                                                 |
| `directories`          | 6.x     | XDG / Win / macOS dirs                                                                   |

Versions audited against crates.io on 2026-05-19 (Addresses NEW-CRIT-1). The v2 pin of `ratatui 0.29.x` + a separate `ratatui-crossterm 0.29.x` was wrong: 0.29 was the last monolithic release (backend reached via `ratatui::backend::CrosstermBackend`); the modular workspace lands at `ratatui 0.30` + `ratatui-crossterm 0.1`. Either pin works for `cargo build` but we adopt 0.30 because tachyonfx 0.24 requires it.

### Event loop (per-client tick)

```rust
loop {
    tokio::select! {
        msg = socket_reader.next() => apply_daemon_event(msg),
        evt = crossterm_event_stream.next() => route_input(evt),
        _   = frame_timer.tick() /* 16.66 ms */ => render_frame(),
        _   = signal_winch.recv() => { reprobe_caps(); send_pty_resize(); }
    }
}
```

We **don't** poll input inside the tick branch; crossterm's async event stream is muxed with the socket reader. This eliminates tick-aligned input lag.

### Render pipeline (per frame, in order)

1. **Tick the local FSM** (`OfficeState::update(dt)`) — applies wander, walk progress, animation phase per character. Deterministic from `worldSeed`.
2. **Compose static layout layers** (immutable per layout-snapshot): tile grid (floor pattern + color), wall base color rect, wall sprites.
3. **Z-sort drawables** (furniture, characters, walls extended, outlines, bubbles).
4. **Rasterize to a back-buffer cell grid** sized to rows×cols of terminal.
5. **Compose chrome on top**: bottom toolbar, top zoom widget, tool overlay, modal (if open), edit action bar (if dirty).
6. **Diff vs previous back-buffer**; emit only changed cells via Ratatui's built-in diff renderer.
7. **PTY pane**: separate sub-region. If `focus == PtyAgent(id)`, the focused agent's `wezterm-term::Terminal` cell grid is rasterized into the PTY pane region. Otherwise PTY is rendered as a low-res preview thumbnail.

### Input handling (incl. MAJ-9 — bracketed paste & mouse)

- Crossterm raw mode; mouse enabled with SGR protocol; `EnableBracketedPaste` on.
- **Focus model**: client maintains `focus: Office | PtyAgent(id) | Editor | Modal`.

| Mode         | Bracketed paste                                                                                                                                               | Mouse                                                                                             |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| Office       | **Disabled.** Pasted text is ignored (Office has no text input).                                                                                              | SGR mouse captured by client for click/hit-test/drag/scroll.                                      |
| PtyAgent(id) | **Passed through to PTY.** Crossterm reports `Event::Paste(s)` → daemon `pty.input { id, bytes: <BPM>s<BPM> }` (full bracketed-paste sequence reconstructed). | Captured by client unless terminal-pane has captured mouse via DEC mode set; then passed through. |
| Editor       | **Disabled.**                                                                                                                                                 | SGR mouse captured by client (paint, drag, etc.).                                                 |
| Modal        | **Disabled.**                                                                                                                                                 | Modal widget consumes mouse events.                                                               |

Special keys reserved for client across all modes (configurable in `~/.pixel-agents/keymap.toml` — Addresses MIN-7): Ctrl+Alt+arrow for pane switch, Ctrl+Alt+Q to quit client, Ctrl+Alt+L to toggle layout editor. Default `Tab` toggles between Office and PtyAgent focus — **but** is overridden to "send literal Tab to PTY" when in PtyAgent mode (Addresses MIN-6: tab-complete in Claude works). Mode-switch in PtyAgent mode uses `Ctrl+Alt+O`.

### Focus indicator

In Office mode, a thin pixel-perfect frame (terminal cells around the embedded canvas region) plus a status-line label. In PtyAgent mode, the PTY pane gets the frame.

### Changelog GIFs (N3) and body font (N6) — what the user actually sees (Addresses NEW-MIN-7)

These two parity items are flagged ⚠ in §22 because terminal rendering imposes hard limits that don't exist in the webview. Per-tier behavior:

**N3 — Animated GIFs in the changelog modal.**

- **T1-K / T1-O (Kitty graphics with id-cached frames)**: full animation. The client uploads each frame as a separate `kittyImageId`, then cycles placement every `frameDurationMs` via a simple Ratatui ticker. Frame rate capped at 10 fps to keep terminal-side decode cheap. Visually equivalent to the webview.
- **T2 (iTerm2 inline images)**: first frame only. iTerm2 supports animated GIFs in inline images natively in 2024+, so on iTerm2 specifically we emit the raw GIF bytes and let the terminal animate. On WezTerm-iterm2-mode the inline path discards animation — first frame only.
- **T3 (Sixel)**: first frame only. Animating Sixel at 10 fps would saturate the terminal parser; we render frame 0 and add a footer line "(animation; open in browser for full)".
- **T4–T6 (half-block / block / braille)**: first frame, downscaled. The user sees a static thumbnail and a footer link.

In every tier the modal renders the changelog text fully; only the embedded GIFs degrade.

**N6 — FS Pixel Sans body text.**

- **T1-K / T1-O / T2 (image tiers)**: headings, toolbars, and the editor action bar are rendered as **sprite text** — pre-rasterized FS Pixel Sans glyph strips composited via Kitty/iTerm2 image placement at the appropriate cell location. Body text (PTY pane content, scrollback, multi-line description blobs) uses the terminal's own font: there is no way to override the body font in a terminal protocol. Net effect: chrome looks pixel-perfect, body text looks like the user's normal terminal — which is what users expect when reading a TUI.
- **T3 (Sixel)**: same split as image tiers; sprite headings via Sixel, body via terminal font.
- **T4–T6 (half-block / block / braille)**: **everything** uses the terminal font. We attempt a rough half-block render of headings at the very largest cell-zoom only on T4 (the result is legible but pixelated); on T5/T6 we surrender and use plain ANSI bold + color for emphasis.

Both items are documented limitations, not bugs; the parity checklist's ⚠ markers reflect the gap to the webview's full-control browser rendering.

---

## 7. Terminal Capability Detection (Addresses MAJ-3)

### Pre-app input queue

Before entering raw mode the client probes capabilities. We launch a small **input drain** thread that reads stdin into a `BytesMut` buffer; the main thread issues escape probes (DA1, Kitty graphics probe, iTerm2 `ReportVariable`, `CSI 14 t`, `CSI 18 t`) and waits up to 150 ms for matching reply prefixes. Any bytes that don't match a known probe-reply prefix are pushed into the **pre-app input queue**, a `VecDeque<KeyEvent>`. On entering raw mode the main event loop drains this queue **before** reading new input — so a user who hit a key during probe gets their input delivered, not eaten.

Parser for the drain thread: `vte 0.13` (lightweight, tolerant of garbage). It produces structured `KeyEvent`s; non-key sequences (replies) are routed to capability state.

### Env-override

`PIXEL_AGENTS_TIER` env var force-overrides probe results:

| Value       | Tier |
| ----------- | ---- |
| `kitty`     | T1   |
| `iterm2`    | T2   |
| `sixel`     | T3   |
| `truecolor` | T4   |
| `256`       | T5   |
| `16`        | T6   |
| `braille`   | T6b  |

Useful for CI snapshots and for users on terminals with broken probe replies.

### Probe ladder (executed in order, short-circuiting)

1. **Read environment first**:
   - `$TERM`, `$COLORTERM` (looking for `truecolor` or `24bit`).
   - `$TERM_PROGRAM` (`iTerm.app`, `ghostty`, `WezTerm`, `vscode`, `Apple_Terminal`).
   - `$KITTY_WINDOW_ID`, `$WEZTERM_PANE`, `$GHOSTTY_RESOURCES_DIR`, `$WT_SESSION` (Windows Terminal), `$VTE_VERSION` (gnome-terminal family).
   - `$TMUX`, `$ZELLIJ`, `$STY` (screen) — for passthrough detection.
2. **Stage 1 (escape probes, parallel)**:
   - DA1 secondary device attributes: `\x1b[c` → mode 4 = Sixel.
   - Kitty graphics probe: `\x1b_Gi=99,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\` → expect `\x1b_Gi=99;OK\x1b\\`.
   - iTerm2 probe: `\x1b]1337;ReportVariable=…` → only iTerm2 / WezTerm-iterm2 mode replies.
   - Unicode placeholder probe (see MAJ-4): `\x1b_Ga=T,i=98,U=1,c=1,r=1,q=2;AAAA\x1b\\` followed by a placeholder cell; we read back DA1 to check the row layout is correct.
3. **Stage 2: color depth** via `tput colors` and `$COLORTERM`. If unknown, write a 24-bit color then read `tput colors`. Conservative: assume 256 if uncertain.
4. **Stage 3: size** via `crossterm::terminal::size()` + `CSI 14 t` for cell pixel sizes; default 8×16 cells if unavailable.

All probes have a 150 ms aggregate timeout. Cached in `~/.pixel-agents/capabilities-cache.json` keyed by `($TERM,$COLORTERM,$TERM_PROGRAM,$WT_SESSION,$KITTY_WINDOW_ID,$WEZTERM_PANE,$TMUX,$ZELLIJ)`, 7-day TTL.

### Fallback ladder (Addresses MAJ-4 + MAJ-5)

```
Best  ┌───────────────────────────────────────────────────────────────────┐
      │ T1-K  Kitty graphics + **unicode placeholders** (Kitty, Ghostty)  │
      ├───────────────────────────────────────────────────────────────────┤
      │ T1-O  Kitty graphics + non-virtual placement (a=T no U=1)         │
      │         (WezTerm, Konsole 22+, foot 1.21+)                        │
      ├───────────────────────────────────────────────────────────────────┤
      │ T2    iTerm2 inline images (iTerm2, WezTerm iterm2 mode)          │
      ├───────────────────────────────────────────────────────────────────┤
      │ T3    Sixel (xterm -ti vt340, WezTerm, foot, mlterm, Win 1.22+)   │
      ├───────────────────────────────────────────────────────────────────┤
      │ T4    24-bit truecolor half-block ▀▄ (Alacritty, gnome-terminal)  │
      ├───────────────────────────────────────────────────────────────────┤
      │ T5    256-color half-block                                        │
      ├───────────────────────────────────────────────────────────────────┤
      │ T6    16-color block ▓ — degraded, character silhouettes          │
      │ T6b   Braille ⠿ — monochrome sub-cell precision                    │
Worst └───────────────────────────────────────────────────────────────────┘
```

**T1 split (Addresses MAJ-4 + NEW-MAJ-2).** v1 lumped Kitty/Ghostty/WezTerm/Konsole as a single Kitty tier with `U=1` unicode placeholders. v2 over-corrected by demoting **all** non-Kitty terminals to T1-O on the basis of a "Ghostty placeholder quirks" claim that did not survive primary-source review. Reality (May 2026):

- **Kitty** ships placeholders as designed; scrollback works correctly.
- **Ghostty 1.3+** fully supports the Kitty unicode-placeholder protocol, including through multiplexers such as tmux — Hashimoto explicitly states Ghostty is the only terminal other than Kitty implementing this part of the spec ([X post, 2024](https://x.com/mitchellh/status/1818696111999299976); [Hachyderm thread](https://hachyderm.io/@mitchellh/112882200482778154); confirmed against [ghostty.org/docs/features](https://ghostty.org/docs/features)). v2's "subtle row-anchoring quirks" wording was factually wrong; we elevate Ghostty back to **T1-K**.
- **WezTerm** supports unicode placeholders but image rows can drift under rapid resize.
- **Konsole 22+** supports the graphics protocol partially; placeholder support is incomplete.

We therefore place Kitty **and Ghostty** at T1-K and keep WezTerm, Konsole 22+, and foot 1.21+ on **T1-O** ("Kitty-others"), using non-virtual placements (`a=T` without `U=1`). T1-O caveat: images don't survive scrollback (they're tied to absolute rows, not cell content). Documented limitation. A **runtime probe** at startup verifies placeholder support and remains the escape hatch if any specific terminal build ships a regression: we emit a placeholder cell, force-scroll one row via `\x1b[1S`, read back the cell with `\x1b[6n` cursor query, and check the image still anchors to the original row. If yes → T1-K; if no → T1-O. So even if a future Ghostty point release breaks placeholders, the probe demotes that host without needing a doc change.

Inside tmux/zellij: if `$TMUX` and not `tmux -CC`, downgrade past T1-K/T1-O/T2 unless `tmux set -g allow-passthrough on` is detected (we issue a probe through DCS passthrough `\ePtmux;\e\eP…\e\e\\\e\\`; if it works, stay; else drop to T3 Sixel which is more reliably passed through). Document tmux 3.4+ with `allow-passthrough on` as recommended.

---

## 8. Rendering Tiers

Common math: TUI cells are roughly 2:1 (h:w). A pixel-art sprite at `zoom = z` occupies `(width_px·z) × (height_px·z)` device pixels. We map device pixels to cells using `cellW × cellH` reported by `CSI 14 t` (default 8×16). At zoom 2 and 8×16 cells, a 32-px-wide sprite is exactly 8 cells wide and 4 cells tall in image tiers.

### Per-tier targets (Addresses MAJ-5)

Frame budgets calibrated against the actual cost of each protocol; v1 over-promised Sixel.

| Tier                            | Visual fidelity     | Frame budget (ms) | Target FPS | Notes                                                                       |
| ------------------------------- | ------------------- | ----------------- | ---------- | --------------------------------------------------------------------------- |
| T1-K Kitty unicode placeholders | Pixel-perfect       | 8                 | 60         | Best-case; image IDs cached, draws are 1-2 ms.                              |
| T1-O Kitty non-virtual          | Pixel-perfect       | 10                | 60         | Slightly more work re-anchoring.                                            |
| T2 iTerm2                       | Pixel-perfect       | 16                | 60         | Base64 overhead; quadrant-dirty mitigation.                                 |
| T3-foot/wez/mlterm              | Sixel pixel         | 30                | 30         | Sixel parsers fast on these.                                                |
| T3-xterm                        | Sixel pixel         | 60                | **15**     | xterm Sixel parser is slow; 30 fps unachievable. (Was over-promised in v1.) |
| T3-WindowsTerminal              | Sixel pixel         | 50                | 20         | WT 1.22+ Sixel reported slow.                                               |
| T4 24-bit ½ block               | Half-cell color     | 4                 | 60         | Trivially cheap.                                                            |
| T5 256-color ½ block            | Half-cell quantized | 4                 | 60         |                                                                             |
| T6 16-color block               | Silhouettes         | 4                 | 60         | Last-resort.                                                                |
| T6b braille                     | Mono sub-cell       | 4                 | 60         | Optional.                                                                   |

### Tier→pixel math

| Tier  | Sprite→cell math                                                                                                                                  | Scrollback safety                                       | Mouse hit-test                        |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- | ------------------------------------- |
| T1-K  | Pixel-exact via `X=,Y=` sub-cell offsets. Image cached on terminal by `i=<id>`.                                                                   | Placeholders show in scrollback.                        | Cell + sub-cell from `CSI 14 t`.      |
| T1-O  | Same image cache; placement uses cell coords only.                                                                                                | Images don't scroll with content — accepted limitation. | Cell + sub-cell.                      |
| T2    | OSC 1337 inline base64 PNG. We split office into 4 quadrants; only dirty quadrants re-emit.                                                       | Inline images scroll with content (acceptable).         | Cell + sub-cell from iTerm2 CellSize. |
| T3    | DCS Sixel pre-quantized per sprite.                                                                                                               | Sixel inside alt-screen only.                           | Cell-only.                            |
| T4    | `\x1b[38;2;...m▀` per cell. At zoom z, 1 cell = 2 sprite rows / 1 sprite col → ⚠ horizontal pixel doubling for square sprites (Addresses MAJ-12). | Trivially safe.                                         | Excellent — clicks map to cells.      |
| T5/T6 | Same geometry as T4 quantized.                                                                                                                    | Safe.                                                   | Same.                                 |

#### F6 (pixel-perfect) status (Addresses MAJ-12)

T1-K, T1-O, T2, T3: **✓** pixel-perfect.

T4/T5/T6: **⚠** — Each half-block cell is ~16 px tall × ~8 px wide (typical terminal). A 1:1 pixel-art sprite at zoom 1 lands 2 vertical pixels per cell-row, but only 1 horizontal pixel per cell-column. The result is **horizontal pixel doubling**: sprites look stretched 2:1 horizontally. We accept this as the cost of half-block rendering. Documented in §22 as ⚠ on F6 for T4-T6 tiers.

Mitigation: render at `effective_zoom = zoom × 2` horizontally only? — rejected because it doubles cache size and the user-perceived visual is identical to "the terminal cell aspect ratio is what it is." Users on half-block tiers get a slightly-wide office. Acceptable.

### Asset pre-processing for tiers (§13 details)

At asset-load time the daemon produces:

- `raw: RGBA Buffer` — source of truth, always.

Lazily, on first use per tier (Addresses MAJ-8):

- T1-K/T1-O: PNG bytes + stable `kittyId` (u32). Daemon includes the id in the world model so all clients reference the same id consistently. PNG bytes transferred once per client per session on `assets.list`.
- T2: base64 PNG (computed once).
- T3: DCS Sixel payload, palette-quantized (per zoom level).
- T4/T5/T6: pre-rasterized cell grid `Vec<(fg, bg, glyph)>` per zoom level.

Cache budget: see the **canonical memory-budget table in §19** (Addresses NEW-MAJ-3). v2 had three different numbers across §8/§13/§19; §19 is now the single source of truth and §8 + §13 cross-reference it.

Summary of the §19 numbers for context here:

- Daemon raw asset cache: ~30 MB.
- Daemon hot tier-blob cache: 10 MB (lazy regen on miss).
- Scrollback rings: 5 agents × 256 KB = **1.25 MB** total (v2 erroneously claimed 30 MB here — the figure was arithmetic, not policy, and is now corrected).
- Client per-tier sprite cache: 50 MB LRU.
- Hue-shifted character variants generated **lazily** on character spawn, not eagerly for all 360°. The 8-position-per-palette wheel pre-cache from v1 is dropped (it ate ~40 MB and most positions were never used).

Combined daemon + 1 client RSS lands at ~180 MB — see §19 for the full breakdown and the R3 revision rationale.

### Hit-testing math

Office content occupies rectangle `(cellX0..cellX1, cellY0..cellY1)`. Mouse click at `(c,r)` → relative cell `(dc,dr)` → world device-pixel `(dc * cellW + subX, dr * cellH + subY)` → world tile `(floor(devX / (TILE_SIZE * zoom)), floor(devY / (TILE_SIZE * zoom)))`. Sub-cell positioning available in image tiers; half-block tiers round to whole-cell.

---

## 9. PTY Hosting & Embedding (Addresses CRIT-1)

### PTY parser strategy: dual-parse with wezterm-term

The **fundamental design decision**: image escape sequences (Kitty graphics protocol, iTerm2 inline images, Sixel) cannot be parsed losslessly by a grid-only terminal emulator — they describe pixel data, not cells. v1 cited `alacritty_terminal` but that crate's image support is non-existent (it discards image escapes), and its `vte::Parser` is annotated as "primarily for use within Alacritty" so its API stability is not guaranteed.

We adopt **`wezterm-term` 0.22+** because:

1. It is **explicitly designed to be embedded** (wezterm's own README: "you provide a std::io::Write implementation that could connect to a PTY, and supply bytes to the model via the `advance_bytes` method"). The crate has no GUI/PTY dependencies.
2. It has **first-class Sixel and iTerm2 inline image cells** — image escapes are parsed into `Cell::Image` with the raw payload retained.
3. It is the parser used by WezTerm itself and is therefore continuously battle-tested against real-world TUI output (including modern Claude Code).
4. It is **maintained**: a stable 0.22.x line ships with wezterm releases; a community fork `tattoy-wezterm-term` exists as a vendoring backstop.

**`vt100-rust` rejected** because: maintenance has slowed (last meaningful commit on `doy/vt100-rust` was 2024; community forks exist but are small) and it has no image cell support.

**`alacritty_terminal` rejected** for the embedding role because: image escape support is absent, API is annotated unstable, embedding documentation is sparse.

### Dual-parse architecture

For Kitty graphics protocol the daemon→client byte stream must support **lossless passthrough**: Kitty places images on the terminal's display surface directly, not into the grid emulator's cells, so the bytes must reach the user's terminal verbatim.

We split the byte stream **upstream of `wezterm-term`** with a **raw-byte tap**:

```
   daemon (PTY raw bytes)
            │
            ▼  binary mux frame
   client receives Bytes
            │
            ├──────────────► raw-byte tap (always-on)
            │                   │
            │                   ├─ if focus == PtyAgent(this id) AND tier == T1-K|T1-O
            │                   │    write verbatim to stdout (image escape passthrough)
            │                   └─ else: drop / mask image escapes
            │
            └──────────────► wezterm-term::Terminal::advance_bytes(bytes)
                                │
                                └─ grid model updated for hit-test + scrollback
```

**Tap interface** (Rust):

```rust
pub trait PtyByteTap: Send {
    /// Called for every chunk of PTY bytes BEFORE wezterm-term sees them.
    /// Returns the bytes the grid parser should consume; the tap may return
    /// a shortened slice to strip image escapes for non-T1 tiers.
    fn intercept<'a>(&mut self, bytes: &'a [u8]) -> &'a [u8];
}

pub struct KittyPassthroughTap {
    tier: Tier,
    focused: bool,
    stdout: BufWriter<Stdout>,
}

impl PtyByteTap for KittyPassthroughTap {
    fn intercept<'a>(&mut self, bytes: &'a [u8]) -> &'a [u8] {
        // Detect Kitty APC: \x1b_G ... \x1b\\
        // Detect iTerm2 OSC 1337: \x1b]1337; ... \x07 or \x1b\\
        // Detect Sixel DCS: \x1bP ... q ... \x1b\\
        if self.tier.supports_kitty_passthrough() && self.focused {
            self.stdout.write_all(bytes).ok();
        }
        // Always return full slice; wezterm-term ignores image escapes it doesn't
        // recognize and parses into Cell::Image those it does (iTerm2 + Sixel).
        // For Kitty placeholders we strip the APC sequences here:
        strip_kitty_apc_if_not_t1(self.tier, bytes)
    }
}
```

This is dozens of lines of Rust, not a research project. The `strip_kitty_apc_if_not_t1` helper is a state machine that recognizes `\x1b_G` … `\x1b\\` boundaries.

### Compilable code sample (replaces v1's non-compiling sample)

```rust
use std::sync::Arc;
use wezterm_term::{Terminal, TerminalConfiguration, TerminalSize};

struct Cfg;
impl TerminalConfiguration for Cfg {
    fn color_palette(&self) -> wezterm_term::color::ColorPalette {
        wezterm_term::color::ColorPalette::default()
    }
}

pub struct PtyClient {
    term: Terminal,
    tap: Box<dyn PtyByteTap>,
}

impl PtyClient {
    pub fn new(cols: u16, rows: u16, tap: Box<dyn PtyByteTap>) -> Self {
        let size = TerminalSize {
            rows: rows as usize,
            cols: cols as usize,
            pixel_width: 0,
            pixel_height: 0,
            dpi: 0,
        };
        let term = Terminal::new(
            size,
            Arc::new(Cfg),
            "pixel-agents",
            "0.1",
            Box::new(std::io::sink()), // we don't write back to PTY here
        );
        Self { term, tap }
    }

    /// Called when daemon delivers a chunk of PTY bytes for this agent.
    pub fn on_pty_data(&mut self, bytes: &[u8]) {
        let to_grid = self.tap.intercept(bytes);
        self.term.advance_bytes(to_grid);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(TerminalSize {
            rows: rows as usize, cols: cols as usize,
            pixel_width: 0, pixel_height: 0, dpi: 0,
        });
    }

    pub fn render_grid(&self) -> impl Iterator<Item = (usize, usize, &wezterm_term::Cell)> {
        let screen = self.term.screen();
        screen.lines.iter().enumerate().flat_map(move |(row, line)| {
            line.cells().iter().enumerate().map(move |(col, c)| (row, col, c))
        })
    }
}
```

The Cargo dependency: ~~`wezterm-term = "0.22"`~~. **Correction (sourcing resolved, 2026-05):** upstream `wezterm-term` was never published to crates.io. The actual dependency is the Tattoy project's published fork — `tattoy-wezterm-term = "=0.1.0-fork.5"` (MIT) — whose crate root is `tattoy_wezterm_term` but exposes the same `Terminal` / `advance_bytes` API used in the sample above (verified by a compile-time smoke in `client/src/pty/mod.rs`). The code sample's `wezterm_term::` paths map 1:1 to `tattoy_wezterm_term::`. See `docs/tui-implementation-plan.md` §6 "Dependency sourcing — RESOLVED" for the full rationale + the upstream-git fallback.

### Daemon side

`node-pty 1.2.0-beta.13` (or stable when promoted; MS-maintained) spawns `claude --session-id <uuid>` with `process.env`, configurable `cwd`, default `cols/rows` from the requesting client's view.

```ts
const pty = nodePty.spawn('claude', [`--session-id`, sessionId, ...maybeFlag], {
  cwd,
  env,
  cols,
  rows,
  encoding: null,
});
pty.onData((buf) => broadcastPtyData(agentId, buf)); // buf: Buffer of raw bytes
pty.onExit(({ exitCode }) => emitAgentExited(agentId, exitCode));
```

PTY bytes are sent over the binary multiplex (§10), **not** base64-in-JSON (Addresses MAJ-2).

### Focus arbitration & resize protocol (Addresses MAJ-6 + NEW-MAJ-4)

When multiple clients are connected, focus on a given agent is **last-focus-wins**. The daemon stores exactly one `focusedClient: ClientId | undefined` per agent. When client B issues `agent.focus { id: 7 }` while client A already holds focus on agent 7:

1. Daemon transfers ownership: `focusedClient[7] = B`.
2. Daemon emits an unsolicited event `agent.focusLost { id: 7 }` to client A, so A's UI can switch to preview rendering and stop sending input.
3. Daemon responds to B's `agent.focus` request with `{ ok: true, previousOwner?: clientId }` — `previousOwner` is set to A's client id when present, omitted otherwise. This lets B's UI surface a transient "took focus from $other" toast if desired.
4. PTY size is then resized to B's reported pane size (debounced 250 ms — see below).

Edge cases:

- **Same client re-focuses**: idempotent; no event emitted, no resize triggered.
- **Focused client disconnects**: `focusedClient[id]` is cleared; no `agent.focusLost` is emitted (nobody to send it to). The next `agent.focus` from any client claims ownership.
- **Race**: two simultaneous `agent.focus` requests are serialized through the daemon's single-threaded event loop; the later-dispatched one wins. Both clients receive a normal response; the earlier one will receive a `agent.focusLost` immediately after.

**PTY resize policy** (unchanged from v2): PTY size follows the focused client, debounced 250 ms. Non-focused clients render their PTY pane as a scaled-down preview (we resample the grid model).

```
client A focuses agent 7 at 120×40
client B focuses agent 7 at 80×24
   → daemon stores per-agent: focusedClient = A; pty.resize(120, 40)
client B does NOT trigger resize; renders an 80×24 preview by:
   - grid is 120×40 (from wezterm-term mirror it keeps)
   - downscale to 80×24 via nearest-neighbor + readability heuristic
   (or: B can elect to be "scaled preview only" mode)
focus storm: user clicks rapidly between A and B
   → resize requests coalesced over 250 ms; only the last winner triggers pty.resize
```

Daemon also keeps a mirror `wezterm-term::Terminal` per agent for `pty.resync` redraw replay.

### Failure modes

| Failure                                 | Handling                                                                                                                                                                                                                     |
| --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `claude` binary missing                 | Daemon spawns with `shell: true`; if exit code reflects ENOENT-equivalent, send `agent.spawnFailed { id, error: "claude not on PATH" }` → client toast.                                                                      |
| PTY zombie                              | `onExit` fires reliably; safety check 30 s after no I/O + `kill(pid, 0)` liveness → mark dead, emit `agent.exited`.                                                                                                          |
| Encoding edge cases                     | Raw bytes; `wezterm-term` handles UTF-8 stream resync.                                                                                                                                                                       |
| Multi-client desync                     | Daemon timestamps each PTY frame; clients with full ring buffers request `pty.resync { id }` → daemon snapshots its mirror `Terminal` and sends a clear + redraw replay over the binary mux.                                 |
| `--resume` binary version mismatch      | Daemon detects exit code 2 from claude with stderr "session format version mismatch"; emits `agent.spawnFailed { reason: "claude_upgraded" }`; client shows "Re-run this session manually." (Addresses CRIT-2 failure path.) |
| Session expired (>30 days, not on disk) | `claude --resume <id>` exits with "unknown session"; daemon drops the agent from `agents.json`, emits `agent.exited { reason: "session_expired" }`. (Addresses CRIT-2.)                                                      |

---

## 10. Wire Protocol (Addresses MAJ-1, MAJ-2)

**Transport**: Unix domain socket (`SOCK_STREAM`) on Linux/macOS; named pipe `\\.\pipe\pixel-agents-<sha1(user@host)>` on Windows. Both bidirectional, byte-stream.

### Channel multiplex

The byte-stream is multiplexed into **two channels** via a single-byte type tag at the start of every frame:

```
+------+--------------------------------------------------------------+
| 0x00 | NDJSON control: one JSON object terminated by \n             |
+------+--------------------------------------------------------------+
| 0x01 | Binary PTY OUTBOUND (daemon→client):                         |
|      |   streamId:u32_be len:u32_be bytes[len]                      |
+------+--------------------------------------------------------------+
| 0x02 | Binary asset blob: assetId:u32_be tier:u8 len:u32_be bytes   |
|      |   (see "asset blob framing" below for chunking)              |
+------+--------------------------------------------------------------+
| 0x03 | Binary PTY INBOUND (client→daemon, large pastes >64 KB):     |
|      |   streamId:u32_be len:u32_be bytes[len]                      |
|      |   For inputs ≤64 KB use NDJSON `pty.input` (Addresses        |
|      |   NEW-MAJ-5).                                                |
+------+--------------------------------------------------------------+
```

**Ordering invariant** (Addresses NEW-MAJ-5 / NEW-MIN-2): inbound PTY bytes (NDJSON `pty.input` and binary 0x03) have **no cross-channel ordering guarantee** with outbound PTY bytes (0x01). The kernel TTY layer is the arbiter, exactly as with any conventional terminal. Within each direction, ordering is FIFO. This matches the user's intuition: when typing into a real terminal, your characters and the program's output interleave according to the kernel's line discipline, not according to wire arrival order.

NDJSON max line: 256 KB (asserted both sides; was 4 MB in v1 — no longer needs to fit PTY payloads). Binary PTY frames are length-prefixed and capped at 1 MB per frame. Sender splits larger writes; receiver assembles.

`streamId` is the agentId (cast to u32; sub-agent negative ids are remapped to `0xFFFFFFFF - subId` for transport).

#### Asset blob framing (0x02) — chunking & EOF (Addresses NEW-MIN-1)

Each 0x02 frame holds at most **1 MB** of blob payload (the same cap as PTY frames). Most sprite blobs fit in a single frame. For blobs larger than 1 MB:

- The sender splits the blob across multiple consecutive 0x02 frames, all sharing the same `assetId`.
- The `tier` byte is overloaded as a finish marker: bit 7 (0x80) of `tier` is **clear** on every non-final frame and **set** on the final frame. The low 7 bits encode the tier id (`0=raw, 1=T1, 2=T2, …`) — 7 bits is more than enough.
- Receiver concatenates payloads in arrival order; sees the high bit; commits the blob to its cache; clears the in-progress buffer. No interleaving of different `assetId`s within the same channel is allowed — the sender writes one blob's frames contiguously before starting another.

Practical upshot: any single sprite at any tier easily fits in one frame (PNGs for our office sprites are typically 1–10 KB; Sixel at most ~80 KB). Multi-frame splits exist for correctness when wall/floor PNG-megasheets are sent verbatim, not as a hot path.

Hard maximum: 16 MB total per asset (16 × 1 MB frames). Blobs exceeding this are rejected at the daemon-side; the producer is expected to pre-resize.

**Authentication**: UUIDv4 token from `~/.pixel-agents/daemon.json`. First message from client must be NDJSON `hello { token, clientVersion, capabilities }`. Daemon validates token with timing-safe compare; on failure, closes socket.

### NDJSON message envelope

```ts
// daemon/src/types/wire.ts — codegen'd to JSON Schema for Rust consumption
export type WireMessage = Req | Res | Evt | Hello | HelloAck;

export interface Req { kind: "req"; reqId: number; method: string; params: unknown; }
export interface Res { kind: "res"; reqId: number; ok: true; data: unknown; }
                    | { kind: "res"; reqId: number; ok: false; error: { code: string; message: string }; }
export interface Evt { kind: "evt"; topic: string; seq: number; ts: number; data: unknown; }
export interface Hello { kind: "hello"; token: string; clientVersion: string; protoVersion: number; capabilities: ClientCapabilities; }
export interface HelloAck {
  kind: "helloAck";
  daemonVersion: string;
  protoVersion: number;
  bootId: string;
  worldSeed: number;
  sessionId: string;
  subscriptions: string[];
  /**
   * Initial world model. Delivered in the same response as the HelloAck —
   * not as a separate event — so clients can begin rendering immediately and
   * never race the first `world.snapshot` evt. (Addresses NEW-MAJ-1.)
   */
  world: WorldSnapshot;
}

export interface ClientCapabilities {
  rendering: "kitty-k" | "kitty-o" | "iterm2" | "sixel" | "truecolor" | "256" | "16" | "braille";
  cols: number; rows: number;
  cellPx: { w: number; h: number };
  bracketedPaste: boolean;
  mouse: boolean;
}
```

`protoVersion` (Addresses MIN-4): integer, currently `1`. Bumped on any breaking wire change. Daemon and client both refuse mismatched majors.

### WorldSnapshot — full TypeScript schema (Addresses MAJ-1)

```ts
export interface WorldSnapshot {
  schemaVersion: 1;
  worldSeed: number; // u32, deterministic FSM seed
  layout: OfficeLayout; // existing type from webview-ui/src/office/types.ts
  assets: {
    catalog: FurnitureCatalogEntry[]; // existing
    characters: { paletteId: number; pngBytes?: never; assetRef: number }[]; // refs via assets channel
    floors: { patternId: number; assetRef: number }[];
    walls: { tileId: number; assetRef: number }[];
  };
  agents: AgentSnapshot[];
}

export interface AgentSnapshot {
  id: number; // negative for sub-agents
  parentId?: number; // for sub-agents
  sessionId: string;
  palette: 0 | 1 | 2 | 3 | 4 | 5;
  hueShift: number; // 0–359
  seatId?: string;
  isExternal: boolean;
  folderName?: string;
  status: 'idle' | 'active' | 'waiting' | 'permission';
  // active tools: tuple-array because Map<>
  activeTools: Array<[toolId: string, toolName: string]>;
  // active sub-tools, parent-keyed:
  activeSubagentTools: Array<[parentToolId: string, Array<[toolId: string, toolName: string]>]>;
  inputTokens: number;
  outputTokens: number;
}
```

`Map<K,V>` → tuple array. `Set<T>` → `T[]`. This is the canonical serialization (Addresses MAJ-1 Maps/Sets requirement).

### Event topics (full table)

| Topic                  | Data type                                                                                          | Notes                                                                                                                                                                                                                                                                |
| ---------------------- | -------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `world.snapshot`       | `WorldSnapshot`                                                                                    | The **initial** world model is delivered inline on `HelloAck.world` (Addresses NEW-MAJ-1). This `world.snapshot` event is emitted **only** on subsequent canonical state changes (layout edit, asset hot-reload, agent registry change). Steady-state rate: < 1/min. |
| `agent.created`        | `AgentSnapshot`                                                                                    |                                                                                                                                                                                                                                                                      |
| `agent.exited`         | `{ id: number; exitCode: number; reason?: "user"\|"session_expired"\|"claude_upgraded"\|"crash" }` |                                                                                                                                                                                                                                                                      |
| `agent.statusChanged`  | `{ id: number; status: "idle"\|"active"\|"waiting"\|"permission" }`                                |                                                                                                                                                                                                                                                                      |
| `agent.toolStart`      | `{ id: number; toolId: string; toolName: string; status: string; runInBackground?: boolean }`      |                                                                                                                                                                                                                                                                      |
| `agent.toolDone`       | `{ id: number; toolId: string }`                                                                   |                                                                                                                                                                                                                                                                      |
| `agent.toolPermission` | `{ id: number; parentToolId?: string }`                                                            |                                                                                                                                                                                                                                                                      |
| `agent.toolsClear`     | `{ id: number }`                                                                                   |                                                                                                                                                                                                                                                                      |
| `agent.subagentStart`  | `{ id: number; parentToolId: string; toolId: string; status: string; toolName: string }`           |                                                                                                                                                                                                                                                                      |
| `agent.subagentDone`   | `{ id: number; parentToolId: string; toolId: string }`                                             |                                                                                                                                                                                                                                                                      |
| `agent.subagentClear`  | `{ id: number; parentToolId: string }`                                                             |                                                                                                                                                                                                                                                                      |
| `agent.tokenUsage`     | `{ id: number; input: number; output: number }`                                                    |                                                                                                                                                                                                                                                                      |
| `agent.matrixEffect`   | `{ id: number; kind: "spawn"\|"despawn"; t0: number }`                                             | `t0` = daemon `Date.now()`. Clients lock effect phase.                                                                                                                                                                                                               |
| `agent.focusLost`      | `{ id: number }`                                                                                   | Sent only to the previously-focused client when another client takes focus on the same agent (Addresses NEW-MAJ-4).                                                                                                                                                  |
| `layout.changed`       | `{ source: "client"\|"file"; layout: OfficeLayout; writerTag?: WriterTag }`                        | (writer-tag — MAJ-11)                                                                                                                                                                                                                                                |
| `assets.updated`       | `{ catalog: FurnitureCatalogEntry[]; dirs: string[] }`                                             | Hot reload.                                                                                                                                                                                                                                                          |
| `settings.updated`     | `{ settings: Settings }`                                                                           |                                                                                                                                                                                                                                                                      |
| `daemon.shuttingDown`  | `{ reason: string }`                                                                               |                                                                                                                                                                                                                                                                      |
| `daemon.log`           | `{ level: "info"\|"warn"\|"error"; msg: string; at: number }`                                      | For N5 debug overlay.                                                                                                                                                                                                                                                |

Every event carries a monotonically increasing `seq` per topic for ordering. The `world.snapshot` `schemaVersion` field allows future evolution (Addresses MIN-4).

### Command catalog (client → daemon)

| Method               | Params                                           | Returns                                        | Purpose                                                                                                                                                                     |
| -------------------- | ------------------------------------------------ | ---------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `hello`              | `HelloParams`                                    | `HelloAck`                                     | Handshake                                                                                                                                                                   |
| `agent.list`         | —                                                | `{ agents: AgentSnapshot[] }`                  | Initial state                                                                                                                                                               |
| `agent.spawn`        | `{ cwd?: string; bypassPermissions?: boolean }`  | `{ id: number; sessionId: string }`            | A1, A2                                                                                                                                                                      |
| `agent.close`        | `{ id: number }`                                 | `{}`                                           | A3                                                                                                                                                                          |
| `agent.focus`        | `{ id: number }`                                 | `{ previousOwner?: clientId }`                 | B5, drives PTY resize policy (MAJ-6). Last-focus-wins; if another client previously held focus, daemon emits `agent.focusLost { id }` to that client (Addresses NEW-MAJ-4). |
| `agent.reassignSeat` | `{ id: number; seatId: string }`                 | `{}`                                           | J4                                                                                                                                                                          |
| `agent.adopt`        | `{ sessionId: string }`                          | `{ id?: number }`                              | A5                                                                                                                                                                          |
| `pty.input`          | `{ id: number; bytes: base64 }`                  | `{}`                                           | B2. Small inputs (≤64 KB) only; larger pastes use binary mux tag 0x03 (Addresses NEW-MAJ-5). No cross-channel ordering with outbound PTY (0x01) — kernel TTY arbitrates.    |
| `pty.resize`         | `{ id: number; cols: number; rows: number }`     | `{}`                                           | B3                                                                                                                                                                          |
| `pty.resync`         | `{ id: number }`                                 | `{}`                                           | Force redraw replay                                                                                                                                                         |
| `layout.get`         | —                                                | `{ layout: OfficeLayout }`                     | K1                                                                                                                                                                          |
| `layout.save`        | `{ layout: OfficeLayout; writerTag: WriterTag }` | `{}`                                           | I11/K1 — writer-tag (MAJ-11)                                                                                                                                                |
| `layout.import`      | `{ layout: OfficeLayout }`                       | `{}`                                           | K8                                                                                                                                                                          |
| `layout.export`      | —                                                | `{ layout: OfficeLayout }`                     | K8                                                                                                                                                                          |
| `layout.setDefault`  | —                                                | `{}`                                           | K9                                                                                                                                                                          |
| `assets.list`        | —                                                | `{ catalog, characters, floors, walls, dirs }` | H1-H3                                                                                                                                                                       |
| `assets.requestBlob` | `{ assetId: number; tier: string }`              | streamed via binary channel (0x02)             | Lazy fetch (MAJ-8). See "asset blob framing" below for chunking + EOF (Addresses NEW-MIN-1).                                                                                |
| `assets.addDir`      | `{ path: string }`                               | `{}`                                           | H10                                                                                                                                                                         |
| `assets.removeDir`   | `{ path: string }`                               | `{}`                                           | H10                                                                                                                                                                         |
| `settings.get`       | —                                                | `{ settings: Settings }`                       | N2                                                                                                                                                                          |
| `settings.set`       | `{ patch: Partial<Settings> }`                   | `{}`                                           | N2                                                                                                                                                                          |
| `hooks.toggle`       | `{ enabled: boolean }`                           | `{ enabled: boolean }`                         | D4                                                                                                                                                                          |
| `daemon.shutdown`    | —                                                | `{}`                                           | O3                                                                                                                                                                          |
| `subscribe`          | `{ topics: string[] }`                           | `{}`                                           | Granular subscription                                                                                                                                                       |

### Rationale for NDJSON over MessagePack/Cap'n Proto (preserved from v1)

NDJSON beats MessagePack/Cap'n Proto on debuggability (we can `nc -U socket | jq`). The control traffic is small (~1 KB/s). PTY traffic — the volume — is binary (Addresses MAJ-2). Future swap to MessagePack is a one-line `Content-Encoding`-style negotiation in `hello.protoVersion`.

---

## 11. Reused vs Rewritten Code (Addresses CRIT-4)

### Phase 0 prerequisite: MessageSender interface lands in the existing repo

**This is a real milestone.** Before any daemon work begins, the following ships through CI on the existing VS Code extension:

#### The `AgentEventSink` interface

```ts
// src/messageSender.ts — NEW FILE, shipped in Phase 0
export interface AgentEvent {
  kind:
    | 'agentCreated'
    | 'agentClosed'
    | 'agentStatus'
    | 'agentToolStart'
    | 'agentToolDone'
    | 'agentToolPermission'
    | 'agentToolsClear'
    | 'agentSubagentStart'
    | 'agentSubagentDone'
    | 'agentSubagentClear'
    | 'agentTokenUsage'
    | 'agentMatrixEffect'
    | 'existingAgents'
    | 'agentRestoreComplete';
  [k: string]: unknown; // shape per kind; see types/agentEvents.ts
}

export interface AgentEventSink {
  /** Broadcast to all connected sinks (today: a single webview). */
  broadcast(event: AgentEvent): void;
  /** Emit targeted at a single agentId. Currently identical to broadcast — */
  /** the daemon implementation can use it to scope subscriptions. */
  emitTo(agentId: number, event: AgentEvent): void;
}

// VS Code implementation: wraps webview.postMessage with a shape adapter.
export class WebviewSink implements AgentEventSink {
  constructor(private webview: vscode.Webview | undefined) {}
  broadcast(event: AgentEvent): void {
    // shape adapter: today's protocol uses { type, ...rest } not { kind, ...rest }
    this.webview?.postMessage(eventToWebviewMessage(event));
  }
  emitTo(_agentId: number, event: AgentEvent): void {
    this.broadcast(event);
  }
}
```

#### File-by-file refactor (Phase 0 scope, exact)

Verified `grep -c "vscode" src/{fileWatcher,agentManager,transcriptParser,timerManager}.ts` on the current repo (2026-05-19):

```
src/fileWatcher.ts:19
src/transcriptParser.ts:3
src/agentManager.ts:15
src/timerManager.ts:4
```

**Total: 41 lines containing `"vscode"`** across the four files. (Addresses NEW-MAJ-6.)

The v1 critique cited "77+" vscode references; v2 cites 41. The delta is a counting-methodology difference, not a code change:

- v1 critique counted every **identifier occurrence** of `vscode` — including the same identifier appearing twice on one line (e.g. `webview: vscode.Webview | undefined; webview = vscode.window.activeTextEditor` would count as 2) and possibly also `webview: vscode.Webview` in `.d.ts`-style multi-arity sites.
- v2 (and this v3) use `grep -c`, which counts **matching lines, not occurrences**. Lines with two `vscode` references count as 1.

Both numbers are correct under their respective definitions. The Phase 0 milestone gate (`grep -n "vscode" … returns zero`) is unambiguous either way: zero lines = zero occurrences.

Plus every public function in those files takes `webview: vscode.Webview | undefined` as a parameter. Verbatim grep output of the offenders (truncated to first 25):

```
src/transcriptParser.ts:2:  import type * as vscode from 'vscode';
src/transcriptParser.ts:95:   webview: vscode.Webview | undefined,
src/transcriptParser.ts:439:  webview: vscode.Webview | undefined,
src/fileWatcher.ts:24:        import * as vscode from 'vscode';
src/fileWatcher.ts:68:        webview: vscode.Webview | undefined;
src/fileWatcher.ts:80:        webview: vscode.Webview | undefined,
src/fileWatcher.ts:174:       webview: vscode.Webview | undefined,
src/fileWatcher.ts:251:       webview: vscode.Webview | undefined,
src/fileWatcher.ts:350:       webview: vscode.Webview | undefined,
src/fileWatcher.ts:374:       const activeTerminal = vscode.window.activeTerminal;
src/fileWatcher.ts:403:       for (const terminal of vscode.window.terminals) {
src/fileWatcher.ts:459:       terminal: vscode.Terminal,
src/fileWatcher.ts:469:       webview: vscode.Webview | undefined,
src/fileWatcher.ts:587:       webview: vscode.Webview | undefined,
src/fileWatcher.ts:780:       webview: vscode.Webview | undefined,
src/fileWatcher.ts:831:       webview: vscode.Webview | undefined,
src/fileWatcher.ts:927:       webview: vscode.Webview | undefined,
src/agentManager.ts:4:        import * as vscode from 'vscode';
src/agentManager.ts:28:       const workspacePath = cwd || vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || os.homedir();
src/agentManager.ts:75:       webview: vscode.Webview | undefined,
src/agentManager.ts:80:       const folders = vscode.workspace.workspaceFolders;
src/agentManager.ts:87:       const terminal = vscode.window.createTerminal({ … });
src/agentManager.ts:285:      context: vscode.ExtensionContext,
src/agentManager.ts:308:      context: vscode.ExtensionContext,
src/agentManager.ts:320:      webview: vscode.Webview | undefined,
src/timerManager.ts:1:        import type * as vscode from 'vscode';
src/timerManager.ts:10:       webview: vscode.Webview | undefined,
```

Phase 0 refactor by file:

- **`src/transcriptParser.ts`** (3 hits). Pure data parsing; the `webview` parameter is used only to post events. Replace parameter with `sink: AgentEventSink`. Replace `webview?.postMessage({...})` with `sink.broadcast({ kind: "...", ... })`. Drop `import type * as vscode`. Estimated 1-day refactor; the file has good test coverage at `src/__tests__/transcriptParser.test.ts`.
- **`src/timerManager.ts`** (4 hits). Same pattern; replace `webview` with `sink`. Estimated 0.5 day.
- **`src/fileWatcher.ts`** (19 hits). Mixed: most are `webview: vscode.Webview | undefined` parameters (same swap). But **lines 374, 403, 459** call `vscode.window.activeTerminal` / `vscode.window.terminals` directly — this is the terminal-adoption mechanism (C7). We abstract those via a `TerminalRegistry` interface:

  ```ts
  export interface TerminalRegistry {
    activeTerminalCwd(): string | undefined;
    listTerminals(): Array<{ name: string; cwd?: string }>;
    associateAgent(id: number, terminalKey: string): void;
  }
  // VS Code impl wraps vscode.window.*; daemon impl uses node-pty handles.
  ```

  Estimated 2-3 days; the dual-mode session detection is fragile. We add fixture-based regression tests before refactor.

- **`src/agentManager.ts`** (15 hits). Most are `webview` parameters (swap to `sink`). Lines 87 (`vscode.window.createTerminal`), 285/308/610 (`context: vscode.ExtensionContext` for workspace state) are the vscode-coupled parts. We introduce:

  ```ts
  export interface AgentRuntime {
    spawnAgent(opts: { sessionId: string; cwd: string; flags: string[] }): Promise<AgentHandle>;
    closeAgent(handle: AgentHandle): Promise<void>;
  }
  export interface AgentStateStore {
    load(): Promise<PersistedAgent[]>;
    save(agents: PersistedAgent[]): Promise<void>;
  }
  ```

  VS Code impl: `AgentRuntime` calls `vscode.window.createTerminal`; `AgentStateStore` reads/writes `context.workspaceState`. Daemon impl: `AgentRuntime` calls `node-pty.spawn`; `AgentStateStore` reads/writes `~/.pixel-agents/agents.json`. Estimated 3-4 days.

#### Phase 0 milestone gate

The refactor is complete when:

- `grep -n "vscode" src/{transcriptParser,fileWatcher,agentManager,timerManager}.ts` returns **zero** matches (modulo a single `import type` in `agentManager.ts` for `ExtensionContext` interface compatibility, scoped to a `// extension-only` block).
- All existing tests pass green on CI.
- The VS Code extension's behavior is observably identical (manual smoke test with the existing parity checklist).

Only **after** Phase 0 ships is the daemon port unblocked.

### Reused-vs-rewritten table (post-Phase 0)

| Source path                                                 | Destination                                                    | Status (post-Phase 0)   | Notes                                                                                                                            |
| ----------------------------------------------------------- | -------------------------------------------------------------- | ----------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `server/src/server.ts`                                      | `daemon/src/hooks/httpServer.ts`                               | **port verbatim**       | No vscode refs today.                                                                                                            |
| `server/src/hookEventHandler.ts`                            | `daemon/src/hooks/eventHandler.ts`                             | **port verbatim**       | Already takes `MessageSender`-style callback.                                                                                    |
| `server/src/constants.ts`                                   | `daemon/src/constants.ts`                                      | **port verbatim**       | Single source already.                                                                                                           |
| `server/src/providers/file/claudeHookInstaller.ts`          | `daemon/src/hooks/installer.ts`                                | **port verbatim**       | Hook script path unchanged.                                                                                                      |
| `server/src/providers/file/hooks/claude-hook.ts`            | `daemon/src/hooks/claudeHookSrc.ts`                            | **port verbatim**       | esbuild target `node18` CJS.                                                                                                     |
| `server/src/provider.ts`, `teamProvider.ts`, `teamUtils.ts` | `daemon/src/hooks/`                                            | **port verbatim**       |                                                                                                                                  |
| `src/transcriptParser.ts` (post-Phase 0)                    | `daemon/src/watching/transcriptParser.ts`                      | **port verbatim**       | Already on `AgentEventSink`.                                                                                                     |
| `src/fileWatcher.ts` (post-Phase 0)                         | `daemon/src/watching/jsonlWatcher.ts`                          | **port near-verbatim**  | `TerminalRegistry` impl swapped: VS Code → daemon (queries `agentRegistry` for adoption).                                        |
| `src/timerManager.ts` (post-Phase 0)                        | `daemon/src/watching/timerManager.ts`                          | **port verbatim**       |                                                                                                                                  |
| `src/agentManager.ts` (post-Phase 0)                        | `daemon/src/agents/agentManager.ts`                            | **port near-verbatim**  | `AgentRuntime` impl swapped: VS Code → node-pty. `AgentStateStore` impl swapped: workspaceState → `~/.pixel-agents/agents.json`. |
| `src/layoutPersistence.ts`                                  | `daemon/src/layout/persistence.ts`                             | **port verbatim**       | + writer-tag (MAJ-11).                                                                                                           |
| `src/configPersistence.ts`                                  | `daemon/src/config/settings.ts`                                | **port verbatim**       |                                                                                                                                  |
| `src/assetLoader.ts`                                        | `daemon/src/assets/loader.ts`                                  | **port + extend**       | Adds lazy tier pre-processing (MAJ-8).                                                                                           |
| `src/types.ts`                                              | `daemon/src/types.ts`                                          | **port**                | Drop `terminalRef: vscode.Terminal`; add `pty` field.                                                                            |
| `webview-ui/src/office/engine/officeState.ts`               | **`client/src/office/state.rs`**                               | **rewrite (Rust port)** | Pure logic, no React. Same algorithm.                                                                                            |
| `webview-ui/src/office/engine/characters.ts`                | **`client/src/office/characters.rs`**                          | **rewrite (Rust port)** | Pure FSM.                                                                                                                        |
| `webview-ui/src/office/engine/gameLoop.ts`                  | **`client/src/office/loop.rs`**                                | **rewrite (Rust port)** | `requestAnimationFrame` → `tokio::time::interval(16.66ms)`.                                                                      |
| `webview-ui/src/office/engine/renderer.ts`                  | **`client/src/render/`**                                       | **rewrite (Rust port)** | Pixel-art primitives ported to Rust tier-specific code.                                                                          |
| `webview-ui/src/office/sprites/spriteCache.ts`              | `daemon/src/assets/pipeline.ts` + `client/src/render/cache.rs` | **split rewrite**       | Daemon raw + lazy tier-key bookkeeping; client per-tier rasterization.                                                           |
| `webview-ui/src/office/sprites/spriteData.ts`               | `daemon/src/assets/sprites.ts`                                 | **port**                | PNG→RGBA shared utility.                                                                                                         |
| `webview-ui/src/office/floorTiles.ts`                       | `daemon/src/assets/floorTiles.ts`                              | **port**                |                                                                                                                                  |
| `webview-ui/src/office/wallTiles.ts`                        | `daemon/src/assets/wallTiles.ts`                               | **port**                |                                                                                                                                  |
| `webview-ui/src/office/colorize.ts`                         | **`client/src/render/colorize.rs`**                            | **rewrite (Rust port)** | Pure math; small.                                                                                                                |
| `webview-ui/src/office/layout/*`                            | **`client/src/office/layout/`**                                | **rewrite (Rust port)** | TileMap, BFS, serializer, catalog. Cross-validated via shared JSON fixtures (§19).                                               |
| `webview-ui/src/office/toolUtils.ts`                        | **`client/src/office/tools.rs`**                               | **rewrite (Rust port)** |                                                                                                                                  |
| `webview-ui/src/office/editor/editorActions.ts`             | **`client/src/editor/actions.rs`**                             | **rewrite (Rust port)** | Per-client edit; commit via `layout.save` RPC.                                                                                   |
| `webview-ui/src/office/editor/editorState.ts`               | **`client/src/editor/state.rs`**                               | **rewrite (Rust port)** |                                                                                                                                  |
| `webview-ui/src/components/*`                               | **`client/src/ui/`**                                           | **rewrite (Ratatui)**   |                                                                                                                                  |
| `webview-ui/src/notificationSound.ts`                       | `daemon/src/audio/sound.ts`                                    | **rewrite**             | Daemon spawns commands; cascade in §15.                                                                                          |
| `webview-ui/src/office/engine/matrixEffect.ts`              | **`client/src/render/matrix.rs`**                              | **rewrite (Rust port)** |                                                                                                                                  |

**Note on "port verbatim" claim:** post-Phase 0 the count of "verbatim" ports is honest because the vscode-coupling lives in the implementation of injected interfaces — not in the file bodies. The §11 v1 phrase "70+ vscode references" is replaced by "0 vscode references in the daemon code paths once Phase 0 lands and the interfaces are injected in main()."

---

## 12. Layout Editor in a TUI

(Preserved from v1 with minor refinements; v1's editor design was strong.)

The editor is the most input-dense feature and must work entirely from keyboard, with optional mouse augmentation.

### Mode entry / exit

`L` (or click bottom-toolbar Layout button) toggles edit mode. Multi-stage Esc unwinds exactly as in webview: furniture pick → catalog deselect → tool tab close → furniture deselect → exit editor.

### Cursor as brush

In edit mode, a **brush cursor** (a 1-tile inverted-color highlight) appears at the last hovered grid cell. Arrow keys move it 1 tile (Shift+arrow = 5 tiles). `Space` performs the active tool's action at the cursor. `Enter` is identical to `Space`.

### Tools (palette)

A right-side panel (toggleable with `P`) lists tools. Numeric shortcuts:

| Key | Tool                        | Behavior                                                                                              |
| --- | --------------------------- | ----------------------------------------------------------------------------------------------------- |
| `1` | SELECT                      | Move cursor; `Space` selects; drag-to-move when on selected furniture (`Shift+arrow` moves selection) |
| `2` | Floor paint                 | `Space` paints; hold `Shift` while pressing arrows = drag-paint                                       |
| `3` | Wall paint                  | Same as floor; toggle add/remove based on first cell of drag                                          |
| `4` | Erase                       | Sets tile→VOID; works in drag mode too                                                                |
| `5` | Furniture place             | Opens furniture catalog; arrow keys navigate; `R` rotates ghost; `T` toggles state                    |
| `6` | Furniture pick (eyedropper) | `Space` on placed furniture copies type+color, switches to tool 5                                     |
| `7` | Floor eyedropper            | Picks pattern+color from cell, switches to tool 2                                                     |

### Furniture catalog modal

`5` opens a Ratatui list widget showing one card per `rotationGroup` (front orientation preferred), grouped by category (Tabs: All / Desks / Chairs / Storage / Electronics / Decor / Wall / Misc). Each card shows a Kitty-protocol miniature (or half-block fallback) of the sprite + name. Arrow keys navigate; `Enter` selects → returns to brush, ghost preview follows cursor.

### Ghost preview

Tint via per-tier overlay (T1-K/T1-O/T2/T3: sprite + colored translucent rect on top; T4/T5/T6: per-cell color modulation). Green = valid, red = invalid (same `canPlaceFurniture()` logic as webview).

### HSBC sliders as adjustable values

Bottom-right panel when a colorable thing is selected (floor cell, wall, furniture). Each row:

```
H ◄  120 ►  ────────●─────────  Colorize [ ]  Clear
S ◄  +0  ►  ─────●────────────
B ◄  +0  ►  ─────●────────────
C ◄  +0  ►  ─────●────────────
```

Focus moves between rows via `J/K` (or up/down arrow inside this widget). `H/L` (or left/right) adjusts by 1; `Shift+H/L` by 10. Mouse: click on `◄`/`►` chars (SGR hit-tested by widget) does the same. Colorize toggle: `C` cycles. Clear: `X` resets. Changes during a single editing burst coalesce into one undo entry via `colorEditUidRef` analog (500 ms idle threshold).

### Drag-paint

- **Mouse**: SGR mouse left-button-down → button-up path, all painted same tool.
- **Keyboard**: `Shift+arrow` enters "drag-paint mode" — every cell traversed painted until Shift released. Terminals that don't emit Shift-up: end drag at first non-arrow key.

### Drag-to-move selected furniture

`Shift+arrow` translates selection; `Enter` commits, `Esc` reverts.

### Delete / Rotate buttons

```
[ Undo (U) ] [ Redo (Ctrl+R / Y) ] [ Save (S) ] [ Reset (Z) ]    [ Rotate (R) ] [ Delete (D) ]
```

Each button is keyboard-bound and mouse-clickable.

### Grid expansion

Hovering 1 tile outside the current grid (cursor + Shift+arrow extends past edge) shows a dashed ghost outline (in TUI: dim cells with `╌` chars or alternating space/`░`). `Space` calls `expandLayout()`. Max 64×64.

### Editor state (per-client)

```rust
struct EditorState {
    tool: Tool,
    brush_pos: (u16, u16),
    ghost: Option<Ghost>,
    selected_furniture_uid: Option<String>,
    palette_index: Option<u32>,
    hsbc: Hsbc,
    colorize: bool,
    undo_stack: VecDeque<EditOp>,
    redo_stack: VecDeque<EditOp>,
    dirty: bool,
}
```

The undo stack is per-client. On `Save`, the daemon's `layout.save` flushes the current layout, and we let the file watcher loop re-apply to all other clients. Conflicts: writer-tag prevents echo (MAJ-11). If another client saved between our load and save, daemon detects mtime change and returns `layout.save` error `STALE_LAYOUT` → client toast offers reload.

### Mapping to checklist Section I

All I1-I15 ✓ — same as v1.

---

## 13. Asset Pipeline (Addresses MAJ-8)

### Load order (daemon startup)

1. Read `~/.pixel-agents/config.json` for `externalAssetDirectories`.
2. Enumerate bundled assets at `<daemon_install>/assets/` (npm package) + each external dir.
3. For each `furniture/<name>/manifest.json` → parse, validate, build `FurnitureCatalogEntry`. External overrides bundled on `id` collision.
4. Load `floors.png` (or per-pattern PNGs), `walls.png`, `char_0..5.png`.
5. Build rotation groups + state groups maps.
6. **Defer** per-tier pre-processing until a client requests it via `assets.requestBlob`.
7. Emit `assets.updated` event.

### Per-tier pre-processing (lazy, MAJ-8)

For each `SpriteData` (after colorize/hue adjust):

```
key = sha1(id || palette || hueShift || hsbcArgs || colorizeFlag)
```

Tier artifacts keyed by `(key, tier, zoom)`:

- T1-K/T1-O: PNG bytes (`Vec<u8>`) + `kittyImageId` assigned by daemon (monotonic u32). Transmitted to client over the binary channel (0x02 type tag) on first reference. Subsequent draws reference by id. **kittyImageId↔sha1 relationship (Addresses NEW-MIN-5):** `kittyImageId` is allocated **lazily** on the first cache miss for the sha1 key (`sha1(id || palette || hueShift || hsbcArgs || colorizeFlag)`), then memoized. Multiple agent spawns producing the same sha1 key share the same `kittyImageId`, conserving terminal-side GPU memory — Kitty stores each image once and re-uses placements across all references.
- T2: base64 PNG (per zoom).
- T3: pre-encoded DCS Sixel (per zoom).
- T4/T5/T6: pre-rasterized cell grid `Vec<(fg, bg, glyph)>` per zoom.

#### Eviction

LRU with hard caps — **all numeric caps live in the canonical memory-budget table in §19** (Addresses NEW-MAJ-3). Policy summary:

- **Daemon**: bounded raw asset cache + a smaller hot tier-blob cache (regenerated on miss). See §19 for the exact MB figures.
- **Client**: per-tier sprite cache, LRU. Evict by (tier, zoom, key); raw images never evicted. See §19 for cap.
- Hue-shifted character variants generated **lazily** on character spawn, cached for that character's lifetime.

### Per-folder manifest

```json
{
  "id": "MONITOR_FRONT_OFF",
  "label": "Monitor",
  "category": "electronics",
  "footprint": { "w": 1, "h": 2 },
  "groupId": "monitor",
  "orientation": "front",
  "state": "off",
  "canPlaceOnSurfaces": true,
  "backgroundTiles": 1
}
```

Identical schema to today. Hot-reload via `chokidar`; debounce 250 ms; rebuild catalog; emit `assets.updated`. Clients dump per-tier cache for affected sprites.

### Character sprites (H12, H13)

Same 112×96 layout as webview. `char_0.png`–`char_5.png`. Hue-shifted variants generated lazily on character spawn — not the v1 "8 positions × 6 palettes pre-cache" which wasted memory.

### Default layout fallback chain (K7)

1. `~/.pixel-agents/layout.json` if exists & valid.
2. Migration from VS Code `workspaceState['pixel-agents.layout']` (one-shot at first daemon start; sentinel `~/.pixel-agents/.migrated`).
3. Bundled `<daemon_install>/assets/default-layout.json`.
4. Procedural `createDefaultLayout()`.

---

## 14. Audio & Notifications (Addresses MAJ-7)

Daemon-side, since clients may be remote (future ssh use case).

### Audio cascade per platform

**Linux** — execute the first command available, fall back to next on failure:

```
1. pw-play <wav>           (PipeWire native — Fedora/Ubuntu/Arch 2024+)
2. paplay <wav>            (PulseAudio or PipeWire-pulse compat)
3. aplay <wav>             (ALSA, last resort)
4. printf '\a'             (terminal bell — distinct from sound; logged WARN)
```

PipeWire shipped as default on every major distro by 2025, but `pw-play` may or may not be in `$PATH` (it's part of `pipewire-utils`). `paplay` is present on most systems either via PulseAudio or via PipeWire's compat shim. The fallback ensures any audio stack works.

**macOS**:

```
1. afplay <wav>            (always present in /usr/bin)
2. osascript -e 'beep 1'   (system bell fallback)
```

**Windows**:

```
1. powershell -NoProfile -Command "(New-Object Media.SoundPlayer '<wav>').PlaySync()"
2. cmd /c "echo ^G"        (BEL via cmd, as last resort)
```

PowerShell startup is slow (~150 ms). We keep one warm-pooled process via `child_process.spawn` with `stdin` pump (existing optimization in webview).

### Cross-window / cross-app notifications

Audio plays only on the host running the daemon. To notify users with the terminal hidden / on a different desktop, we also emit a desktop notification:

| OS      | Command                                                                                                                                                                                | Fallback |
| ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------- |
| Linux   | `notify-send "Pixel Agents" "Agent N is waiting"`                                                                                                                                      | bell     |
| macOS   | `terminal-notifier -title "Pixel Agents" -message "Agent N is waiting"` (Homebrew); falls back to `osascript -e 'display notification "..."'`                                          | bell     |
| Windows | `powershell -Command "[reflection.assembly]::loadwithpartialname('System.Windows.Forms'); ...show toast..."` or [BurntToast](https://github.com/Windos/BurntToast) module if installed | bell     |

Notifications fire on `agentStatus: 'waiting'` or `agentStatus: 'permission'` events (and _only_ when the daemon's foreground client is not focused on that agent — to avoid double-notify with the in-TUI bubble). Setting `notificationsEnabled` (M2) in `config.json`; toggle in Settings.

Notification rate-limit: max 1/agent/5s.

Setting `soundEnabled` (M2). Toggle in Settings. Setting `--sound-cmd` daemon flag overrides cascade entirely.

Cross-platform parity (M3): all three resolved.

---

## 15. Hooks Dual-Mode

Identical to today, with hostname-only changes.

```
claude (PTY child)                            daemon
   │                                            │
   │ tool starts                                │
   ├── PreToolUse hook fires ───────────────────┤
   │   → executes ~/.pixel-agents/hooks/claude-hook.js
   │     (CJS, no deps; reads stdin, POSTs)
   │   → POST http://127.0.0.1:<port>/api/hooks/claude
   │     Authorization: Bearer <token>
   │     Body: { hook_event_name, session_id, ...payload }
   │
   │   → daemon httpServer.handleHookRequest
   │        → hookEventHandler.handleEvent
   │           → maps session_id → agentId via sessionToAgentId
   │           → routes by AgentEvent.kind to AgentEventSink
   │           → broadcasts agent.toolStart / agent.toolPermission / agent.statusChanged
   │
   ├── all subscribed clients receive event ────┤
   │   → render bubble / status / sub-agent character
```

### Discovery (Addresses CRIT-5)

Hook script `claude-hook.js` reads, in order:

1. `$PIXEL_AGENTS_HOOK_URL` env var (testing/debug short-circuit).
2. `~/.pixel-agents/daemon.json` if exists, PID alive, `bootId` present.
3. `~/.pixel-agents/server.json` (the VS Code extension's file, for cooperative mode).

This makes the hook script robust to either or both servers being up. The hook server in each process registers the same hook endpoint (`/api/hooks/claude`); whichever the hook script reaches first wins.

Cold-start retry inside the hook script: if both file reads fail, retry once with a 100 ms backoff, then drop the event silently (log to stderr only).

### Auth token rotation policy (Addresses MIN-3)

The auth token in `daemon.json` is generated once per daemon **install** (not per boot). Stored in `daemon.json`; rotated only on:

- Explicit `pixel-agents --rotate-token`.
- Postinstall first-run if `daemon.json` doesn't exist.
- File corruption (token field missing).

Rotation does **not** invalidate active clients because they hold an in-memory copy from `helloAck`. New hook scripts spawned after rotation read the new token. Concurrency: if `pixel-agents --rotate-token` runs while hook events are in flight, a small window (≤500 ms) of 401-rejected hook events is possible; daemon logs WARN.

### Multi-window safety

If a second daemon is launched accidentally, the second sees a live PID + valid port and exits with `EALREADY`. Cooperative-with-extension mode (CRIT-5 resolution) explicitly opts out of this exit.

Pre-registration buffering (D5): exactly current behavior — events without registered agent are held up to `HOOK_EVENT_BUFFER_MS` (constant from `server/src/constants.ts`).

### tmux passthrough probe nesting (Addresses MIN-2)

When `$TMUX` is set we issue the Kitty probe wrapped in DCS passthrough: `\ePtmux;\e\eP\e_Gi=99,...\e_Gi=99;OK\e\\\e\\`. If `tmux set -g allow-passthrough on` is enabled, we receive `\e_Gi=99;OK\e\\` back; otherwise we see the literal escape on screen and our probe times out. When _nested_ (tmux inside tmux), each layer needs `allow-passthrough`. We probe each layer by counting `\ePtmux;` prefixes in the outgoing stream and warning if `$TMUX` has nested sessions visible (`tmux list-sessions` from within).

---

## 16. Persistence & Multi-Client (Addresses MAJ-11, CRIT-2)

### Files (all under `~/.pixel-agents/`)

| File                       | Format               | Writer               | Atomic                | Watched                           |
| -------------------------- | -------------------- | -------------------- | --------------------- | --------------------------------- |
| `daemon.json`              | JSON                 | daemon               | tmp+rename, 0600      | by other daemons (existence test) |
| `server.json`              | JSON                 | VS Code extension    | tmp+rename, 0600      | by daemon (CRIT-5 cooperative)    |
| `socket`                   | UDS                  | daemon               | —                     | —                                 |
| `layout.json`              | JSON v1 + writer-tag | daemon               | tmp+rename            | yes (fs.watch + 2 s poll, K4)     |
| `config.json`              | JSON                 | daemon               | tmp+rename            | yes                               |
| `agents.json`              | JSON                 | daemon               | tmp+rename            | no (read on boot)                 |
| `hooks/claude-hook.js`     | CJS bundle           | esbuild install step | replace+chmod 0700    | no                                |
| `logs/daemon-YYYYMMDD.log` | NDJSON               | daemon               | append                | no                                |
| `capabilities-cache.json`  | JSON                 | client               | tmp+rename per-client | no                                |

### Logging format/path (Addresses MIN-5)

`~/.pixel-agents/logs/daemon-YYYY-MM-DD.log`. Format: one NDJSON object per line:

```json
{
  "ts": "2026-05-19T12:34:56.789Z",
  "level": "info",
  "module": "agents",
  "agentId": 7,
  "msg": "..."
}
```

Rotated daily; gz-compressed after 7 days; deleted after 30 days. Client logs land at `~/.pixel-agents/logs/client-PID-YYYY-MM-DD.log`. Configurable `logLevel: "trace"|"debug"|"info"|"warn"|"error"` in `config.json` (default `info`).

### Layout file format with writer-tag (Addresses MAJ-11)

```json
{
  "version": 1,
  "cols": 20,
  "rows": 11,
  "tiles": [...],
  "furniture": [...],
  "tileColors": [...],
  "_writer": {
    "processId": 12345,
    "bootId": "f81d4fae-7dec-11d0-a765-00a0c91e6bf6"
  }
}
```

The `_writer` field is appended at the end of every write. On file watcher events, the daemon parses the JSON, compares `_writer.bootId` to its own `bootId`:

- match → it's our own write, silently ignore.
- mismatch → it's external (another daemon, manual edit, etc.); apply and broadcast.

This eliminates the timestamp race in v1 (where `markOwnWrite()` used wall-clock proximity). The writer-tag is robust to filesystem clock drift, hibernation, and concurrent writes.

### Save coalescing

Layout writes from clients are debounced 500 ms on the daemon. Writer-tag semantics replace `markOwnWrite()` (K5).

### Conflict resolution (K6 — last-save-wins)

When `layout.changed` arrives from the file watcher (external write — a manual edit, a second daemon if cooperative-with-extension, etc.), daemon checks: if any connected client has unsaved edits in its editor, daemon **does not** broadcast a `layout.changed` event for ≥10 s of editor activity. The next save will overwrite (last-save-wins). When all clients are clean, external change is applied and broadcast.

**Suppression policy (Addresses NEW-MIN-3)**: if **any** connected client has unsaved editor edits, an external `layout.changed` is suppressed for **all** clients. When all clients are clean, the external change is applied and broadcast. (v2 phrased this as "per-client globally guarded" which conflated the per-client dirty-state input with the global suppression output — the wording above is the canonical statement.)

### Multi-client (L1-L4)

- L1: socket accepts unbounded clients (memory cap = `MAX_CLIENTS` = 8 by default, configurable).
- L2: `layout.save` → daemon applies → broadcasts `layout.changed` (with writer-tag) to _other_ clients.
- L3: each client maintains its own `EditorState`, viewport zoom, pan, focus. Daemon stores none of this.
- L4: hooks server unaffected (already multi-window safe).

### Per-cwd agent persistence (K3)

The current extension uses `workspaceState`. TUI substitutes **cwd**. `agents.json` schema:

```json
{
  "version": 1,
  "byCwd": {
    "/home/dale/myproj": [
      {
        "id": 1,
        "sessionId": "a1b2c3d4-...",
        "isExternal": false,
        "palette": 2,
        "hueShift": 0,
        "seatId": "uid:0",
        "jsonlFile": "/home/dale/.claude/projects/-home-dale-myproj/a1b2c3d4-....jsonl",
        "spawnFlags": [],
        "lastSeenAt": 1715000000000
      }
    ]
  }
}
```

### Agent restoration on daemon boot (Addresses CRIT-2 A4 fix)

This is the core of the **A4 → ✓** path.

```ts
// daemon/src/agents/resume.ts (pseudocode)
async function restoreAgentsOnBoot(): Promise<void> {
  const persisted = await readAgentsJson();
  for (const [cwd, entries] of Object.entries(persisted.byCwd)) {
    for (const entry of entries) {
      try {
        await restoreOne(cwd, entry);
      } catch (err) {
        log.warn(`restore failed for ${entry.id}`, err);
        // remove from persistence — see "failure paths" below
      }
    }
  }
}

async function restoreOne(cwd: string, entry: PersistedAgent): Promise<void> {
  // Step 1: JSONL liveness gate.
  // The session must exist on disk and not be marked deleted.
  const jsonlPath = entry.jsonlFile;
  if (!fs.existsSync(jsonlPath)) {
    throw new Error(`jsonl missing: ${jsonlPath}`);
  }
  const stat = await fs.promises.stat(jsonlPath);
  if (Date.now() - stat.mtimeMs > SESSION_STALE_MS /* 30 days */) {
    throw new Error('jsonl stale (>30 days)');
  }

  // Step 2: respawn via --resume.
  const pty = nodePty.spawn('claude', ['--resume', entry.sessionId, ...entry.spawnFlags], {
    cwd,
    env: process.env as any,
    cols: 80,
    rows: 24,
    encoding: null,
  });

  // Step 3: bind hooks by sessionId (same as fresh spawn).
  agentRegistry.register({ ...entry, pty, pid: pty.pid });

  // Step 4: detect early failure.
  // claude --resume exits ~1s if session is unrecognized.
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(resolve, RESUME_HEALTH_TIMEOUT_MS); // 3s
    pty.onExit(({ exitCode }) => {
      clearTimeout(timer);
      if (exitCode !== 0) reject(new Error(`claude --resume exited ${exitCode}`));
      else resolve();
    });
  });

  // Step 5: emit agent.created so clients spawn the character (with skipSpawnEffect: false).
  bus.broadcast({
    kind: 'agentCreated',
    id: entry.id,
    sessionId: entry.sessionId,
    palette: entry.palette,
    hueShift: entry.hueShift,
    seatId: entry.seatId,
    isResumed: true,
  });
}
```

#### Failure paths spec (Addresses CRIT-2)

| Failure                                                                                                      | Detection                                                 | Action                                                                                                  |
| ------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| JSONL file missing                                                                                           | `fs.existsSync(jsonlPath) === false`                      | Drop entry from `agents.json`; log INFO.                                                                |
| JSONL stale (>30 days)                                                                                       | `mtimeMs` check                                           | Drop entry from `agents.json`; log INFO.                                                                |
| `claude` binary not on PATH                                                                                  | `pty` ENOENT immediately                                  | Keep entry; emit `agent.spawnFailed { reason: "claude_missing" }` on next client connect; client toast. |
| `claude --resume` returns "unknown session"                                                                  | exit code != 0 within 3 s                                 | Drop entry; log INFO.                                                                                   |
| `claude` binary version mismatch (older daemon resumed under newer claude with breaking JSONL format change) | exit code 2 with stderr "session format version mismatch" | Keep entry but mark `needsManualResume: true`; client UI shows "Re-run manually" prompt.                |
| `claude --resume` hangs (network, auth)                                                                      | 30 s no JSONL activity                                    | Keep PTY alive; standard timer logic takes over.                                                        |
| Concurrent multi-cwd entries                                                                                 | Loop is serial; no concurrency issues.                    | —                                                                                                       |

This achieves **A4 ✓** under CRIT-2 resolution (b) (`--resume`) combined with the OS supervisor for daemon restarts. Both are needed because:

- Supervisor without `--resume` would respawn the daemon but the daemon could not resurrect dead PTYs.
- `--resume` without supervisor means a daemon crash leaves no one to relaunch agents.

### Migration UX (Addresses §22 #8)

On first daemon boot we check for `~/.config/Code/User/workspaceStorage/<hash>/...pixel-agents-extension` (VS Code's workspace state directory). If present and no `~/.pixel-agents/agents.json` exists, we run a **silent** migration (the old behavior). If the user wants prompts, they can pass `--migrate=prompt`. Sentinel file `~/.pixel-agents/.migrated` prevents repeat runs.

---

## 17. Distribution

### Packaging

**Single npm package**: `pixel-agents`. Contains:

- `dist/daemon/` — TypeScript-compiled daemon entrypoint + assets.
- `dist/hooks/claude-hook.js` — bundled CJS.
- `bin/pixel-agents` — Node launcher that prefers a sibling Rust binary.
- `bin/pixel-agents-tui-<platform>-<arch>` — Rust client binary (downloaded postinstall from GitHub Releases keyed by `pixel-agents` version).
- `share/supervisors/` — `systemd.service`, `launchd.plist`, `scheduled-task.xml` (§4).

Postinstall: detect `process.platform`/`process.arch`, fetch matching binary tarball, verify **sha256 against shipped manifest** (`bin/manifest.json` in npm tarball), extract to `bin/`. The shipped manifest is committed in the npm package and is the _only_ source of truth — postinstall never trusts a remote-only sha (Addresses MAJ-10).

Install:

```sh
npm install -g pixel-agents
pixel-agents                  # starts daemon + client
pixel-agents --install-supervisor  # optional: install systemd/launchd/task
```

Alternatives:

- `cargo install pixel-agents-tui` — installs Rust client only; user must install daemon via npm.
- Homebrew formula `brew install pixel-agents` — **post-MVP roadmap; formula to be published after v1.0 ships** (Addresses NEW-MIN-6). MVP install path is npm only; Homebrew is a tracking item, not a shipping artifact.
- Linux AUR / .deb / .rpm — community-maintained; out of scope MVP.

### Naming (Addresses MIN-9)

| Name                            | Role                                                                                      |
| ------------------------------- | ----------------------------------------------------------------------------------------- |
| `pixel-agents`                  | The npm package and the launcher binary.                                                  |
| `pixel-agents-daemon`           | The daemon entrypoint (when invoked as `pixel-agents --daemon`). Not its own binary name. |
| `pixel-agents-tui`              | The Rust TUI client binary.                                                               |
| `pixel-agents-vscode-extension` | The existing VS Code extension on the marketplace.                                        |

The user-facing command is always `pixel-agents`. Internal subcommands: `--daemon`, `--tui` (forced), `--rotate-token`, `--install-supervisor`, `--migrate`, `--no-update-check`.

### Single binary vs split

We split because: (a) the Rust client doesn't need 250 MB of Node, (b) Node daemon is the natural home for the existing codebase + hot-restart workflow, (c) `gh release` artifacts are easier per-platform than a unified bundle.

### Auto-update (Addresses MAJ-10)

**Default: off.** Auto-update is opt-in via `config.json` `autoUpdate: "off" | "check"`. **There is no `"apply"` mode.** The daemon never self-upgrades in place — too many security and reliability hazards (downgrade attacks, partial-install on crash, dependency drift mid-session).

`autoUpdate: "check"` (only enabled by explicit user action):

- Daemon checks `https://api.github.com/repos/pablodelucca/pixel-agents/releases/latest` once on startup, with 24 h backoff.
- If a newer release exists: emit `settings.updated { availableVersion: X }` → clients show a **banner** with the version and a link to release notes.
- The user runs `npm install -g pixel-agents@latest` themselves to upgrade.

The Rust binary tarball download verifies the sha256 against the **npm-resolved** package manifest only. The daemon never fetches a tarball from any URL not anchored in the npm registry's package.json. (Addresses MAJ-10's "verify SHA against npm-resolved package only".)

---

## 18. Testing

### Unit tests

| Suite                    | Tool                 | Scope                                                                                                                             |
| ------------------------ | -------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| Daemon protocols         | Vitest               | RPC dispatch, framing (NDJSON + binary mux), auth, error paths                                                                    |
| Daemon agents            | Vitest               | PTY lifecycle (mock node-pty), `--resume` restore, sessionId mapping, failure paths (CRIT-2)                                      |
| Daemon hooks             | Vitest               | Reuse all current `server/__tests__/`                                                                                             |
| Daemon transcript parser | Vitest               | Reuse fixtures; new for agent_progress/bash_progress/mcp_progress                                                                 |
| Daemon timers            | Vitest               | Permission 7 s, text-idle 5 s, suppressed-by-hook                                                                                 |
| Daemon layout            | Vitest               | Atomic write, writer-tag round-trip, conflict resolution, fallback chain                                                          |
| Daemon supervisor        | shell                | systemd-user / launchd / Scheduled Task install scripts pass `--check`                                                            |
| Client capability        | cargo test           | Mock-stdout terminal: assert correct tier given env vars + DA1 replies; `PIXEL_AGENTS_TIER` override                              |
| Client rendering         | cargo test + `insta` | One snapshot per tier × scene                                                                                                     |
| Client editor            | cargo test           | Editor ops; undo/redo invariants                                                                                                  |
| Client FSM               | cargo test           | Deterministic from `worldSeed`: same seed + same event stream = identical character positions tick-for-tick (CRIT-3 verification) |
| Pathfinding parity       | shared JSON fixtures | TS test in webview + Rust test in client both consume `pathfinding-fixtures.json` and must produce identical BFS results          |

### Snapshot tests (P3)

`insta` for the entire emitted byte stream from Ratatui under a given fixture state. Diffing via `insta --review`. Pinned terminal dimensions per snapshot (80×24, 120×40, 200×60).

### E2E (P4)

Spawn a real `claude` via the daemon, drive a scripted prompt, assert:

1. `agent.created` event arrives.
2. `agent.toolStart` for Write arrives within 5 s.
3. `agent.toolDone` arrives.
4. `agent.statusChanged → waiting` within turn_duration window.

Requires `CLAUDE_API_KEY` or our ship-with-mock `claude-mock` binary that replays a prerecorded JSONL stream.

### Terminal compatibility matrix (P5)

| Terminal                | Image                     | Tier                                                                   |
| ----------------------- | ------------------------- | ---------------------------------------------------------------------- |
| Kitty 0.36+             | `kovidgoyal/kitty:latest` | T1-K                                                                   |
| Ghostty 1.3+            | `ghostty:latest`          | T1-K (Addresses NEW-MAJ-2)                                             |
| WezTerm 20240210+       | `wezterm/wezterm:nightly` | T1-O                                                                   |
| Alacritty 0.14+         | `alacritty:latest`        | T4                                                                     |
| foot 1.21+              | `dnkl/foot:latest`        | T3                                                                     |
| xterm with `-ti vt340`  | `xterm:vt340`             | T3 (15 fps target)                                                     |
| gnome-terminal          | `gnome-terminal` (Xvfb)   | T4                                                                     |
| Windows Terminal 1.22+  | Win runner (PR-only)      | T3 (20 fps target)                                                     |
| Apple Terminal          | macOS runner              | T4                                                                     |
| Tmux 3.4 + Kitty        | composed                  | T1-K with passthrough on                                               |
| Tmux 3.4 + Ghostty 1.3+ | composed                  | T1-K (Ghostty unicode placeholders survive tmux — Addresses NEW-MAJ-2) |
| Tmux 3.4 + WezTerm      | composed                  | T1-O with passthrough on                                               |

For each, run an automated session via `expect` / `pexpect` driving keystrokes and capturing the framebuffer (`tmux capture-pane -p`) to verify content.

---

## 19. Performance Budget (revised per MAJ-5, MAJ-8)

### Frame budget targets

- T1-K Kitty: ≤16 ms per frame at 60 fps.
- T1-O: ≤16 ms.
- T2 iTerm2: ≤16 ms; degrade to 30 fps if >20 ms three frames in a row.
- T3 foot/WezTerm/mlterm: ≤33 ms at 30 fps.
- T3 xterm: ≤66 ms at 15 fps. (Revised down — xterm Sixel parser is slow.)
- T3 Windows Terminal 1.22+: ≤50 ms at 20 fps.
- T4-T6 half-block: ≤8 ms at 60 fps trivially.

### Hot paths (per frame, T1-K)

| Stage                                      | Estimated cost | Optimization                        |
| ------------------------------------------ | -------------- | ----------------------------------- |
| Read socket buffered events                | <1 ms          | tokio mpsc behind socket reader     |
| Apply WorldSnapshot delta / local FSM tick | 1-2 ms         | per-character; ≤20 agents           |
| Z-sort drawables                           | <0.5 ms        | `Vec::sort_by_key`; capacity reused |
| Compose cell grid (chrome only)            | 1-2 ms         | Ratatui's built-in diff             |
| Emit Kitty placements                      | 1-3 ms         | Image IDs cached on terminal        |
| Diff vs back-buffer                        | 0.5 ms         | Ratatui                             |
| Stdout write                               | 1-2 ms         | `BufWriter` flushed once            |

Idle (no animation, no PTY data): we skip redraw entirely (event-driven). With 5 idle agents wandering (G5) at 0.5 Hz step rate, expected CPU <2%.

### Memory targets — canonical table (Addresses NEW-MAJ-3)

This table is the **single source of truth** for memory budgets. §8 and §13 reference these numbers; if you see a contradicting figure elsewhere, this table wins.

| Bucket                                           | Cap                                  | Where it lives | Notes                                                                                                  |
| ------------------------------------------------ | ------------------------------------ | -------------- | ------------------------------------------------------------------------------------------------------ |
| Node 22 baseline                                 | ~40 MB                               | daemon RSS     | Empty Node process + V8 + stdlib.                                                                      |
| Asset raw (RGBA) cache                           | ~30 MB                               | daemon RSS     | Every sprite kept decoded; ~150 sprites at typical sizes.                                              |
| Hot tier-blob cache (PNG/Sixel/half-block bytes) | 10 MB                                | daemon RSS     | LRU; regenerated on miss from raw.                                                                     |
| Scrollback rings                                 | **1.25 MB**                          | daemon RSS     | 5 agents × 256 KB each. (v2 erroneously cited 30 MB in §8 — that was wrong arithmetic, now corrected.) |
| Daemon misc / V8 overhead                        | ~5 MB                                | daemon RSS     | Heap fragmentation, sockets, hook server.                                                              |
| **Daemon total**                                 | **~85 MB nominal, hard cap 100 MB**  |                |                                                                                                        |
| Rust client baseline                             | ~30 MB                               | client RSS     | Tokio + ratatui + crossterm + wezterm-term mirror per agent.                                           |
| Per-client sprite tier cache                     | 50 MB LRU                            | client RSS     | Current scene only; raw images never evicted.                                                          |
| `wezterm-term` per-agent grid mirror             | ~5 MB                                | client RSS     | 5 agents × ~1 MB grid+scrollback.                                                                      |
| Client misc                                      | ~5 MB                                | client RSS     | Input queue, buffers, FSM state.                                                                       |
| **Client total**                                 | **~90 MB nominal, hard cap 100 MB**  |                |                                                                                                        |
| **Combined (daemon + 1 client)**                 | **~175 MB nominal, hard cap 200 MB** |                |                                                                                                        |

**R3 revision** (Addresses MAJ-8): the original R3 target of <100 MB combined was unrealistic for image tiers. We revise R3 to **<200 MB combined for one daemon + one client** and document in §22 that the original number was over-promised. R3 status: ⚠ (revised target met).

### CPU budget (R4)

5 idle agents: <5% CPU on M1 / Linux mid-tier hardware.

---

## 20. Cross-Platform Notes

### Linux (Q1)

- First-class: Kitty + Ghostty 1.3+ (T1-K, Addresses NEW-MAJ-2), WezTerm/foot/Konsole 22+ (T1-O), recent gnome-terminal (T4).
- Alacritty (T4), xterm -ti vt340 (T3, 15 fps).
- Wayland & X11 both fine.
- UDS at `~/.pixel-agents/socket`; mode 0600.
- Audio cascade: pw-play → paplay → aplay (§15).

### macOS (Q2)

- iTerm2 — T2 (or T1-O via Kitty graphics in 2026+ iTerm2 if user enables it).
- Kitty + Ghostty 1.3+ — T1-K. WezTerm — T1-O. (Addresses NEW-MAJ-2.)
- Apple Terminal — T4. No Sixel, no graphics, ANSI color only.
- node-pty requires `xcode-select --install` once; postinstall checks.

### Windows (Q3)

- Windows Terminal 1.22+ — T3 (Sixel, late 2024+) at 20 fps target; otherwise T4.
- conhost.exe (legacy) — T6 fallback.
- ConPTY for node-pty (built-in on Win 10 1809+). Named pipe at `\\.\pipe\pixel-agents-<sha>`. SACL: current user only.
- Crossterm handles `ENABLE_VIRTUAL_TERMINAL_PROCESSING`.
- node-pty Windows quirks (ConPTY mishandled sequences — §22 #6) handled by a stripping shim if telemetry indicates need.

### Tmux / Zellij / Screen (Q4)

| Multiplexer   | Behavior                                                                                                                                           |
| ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| **tmux 3.4+** | T1-K/T1-O pass through only with `allow-passthrough on`. We auto-detect and probe-check; if off, drop to T3 → T4. Warning toast suggests enabling. |
| **tmux <3.4** | No passthrough; force T4.                                                                                                                          |
| **zellij**    | Native Sixel and Kitty support in 0.40+; otherwise T4. Detect via `$ZELLIJ`.                                                                       |
| **screen**    | T4 forced; document.                                                                                                                               |

Known broken terminals (avoid T1 claims): VS Code integrated terminal pre-1.92, JetBrains terminals (pre-2024.2), default macOS Terminal, mintty-without-WSL.

---

## 21. Parity Checklist Status (revised)

Each item: status + section that addresses it.

### A. Agent Lifecycle

- A1 (MVP) Spawn agent — **✓** §5, §9, §10 (`agent.spawn`).
- A2 (MVP) `--dangerously-skip-permissions` — **✓** §10 (`bypassPermissions` param).
- A3 (MVP) Close agent — **✓** §4, §10 (`agent.close`, `agent.exited`).
- A4 (MVP) **Restore agents on relaunch** — **✓** (was ⚠ in v1). §4 supervisor + §16 `claude --resume` (Addresses CRIT-2).
- A5 (Full) Terminal adoption — **✓** §15, §10, §11.
- A6 (Full) `/clear` detection — **✓** §11.
- A7 (Full) `/resume` detection — **✓** §11, §15.

### B. Terminal / PTY Integration

- B1 (MVP) Each agent has its own PTY — **✓** §9.
- B2 (MVP) User types into focused agent — **✓** §6, §10.
- B3 (MVP) Resize tracking — **✓** §9 (follow-focused-client policy, MAJ-6), §10.
- B4 (MVP) Scrollback per agent — **✓** §5 (ring 256 KB), §9 (wezterm-term grid + scrollback).
- B5 (Full) Click-to-focus character — **✓** §6, §10 (`agent.focus`).
- B6 (Full) Sub-agent click → parent terminal — **✓** §6.
- B7 (Full) Graceful PTY death — **✓** §4.

### C. Agent Status Tracking

- C1-C11 — **✓** §11 (Phase 0 refactor preserves all dual-mode logic verbatim post-interface).

### D. Hook Script & Installation

- D1-D5 — **✓** §15, §17.

### E. Sub-agents

- E1-E6 — **✓** §5 (client-side FSM with shared seed), §11.

### F. Visual Office Rendering

- F1-F5 — **✓** §8.
- F6 (MVP) **Pixel-perfect integer zoom** — **✓ in T1-K/T1-O/T2/T3; ⚠ in T4/T5/T6** (horizontal pixel doubling — Addresses MAJ-12). Documented limitation; not a blocker for MVP because MVP requires only "at least one tier achieves pixel-perfect" — T1-K does. Half-block tiers accept the 2:1 horizontal stretch.
- F7 (Full) Bubbles — **✓** §10 (events), §8.
- F8 (Full) Tool overlay — **✓** §6.
- F9 (Full) Selection outline — **✓** §8.
- F10 (Full) Matrix spawn/despawn 0.3 s — **✓** §5 (`agent.matrixEffect` event with `t0`).
- F11 (Full) Camera follow — **✓** §6.
- F12 (Full) Middle-mouse pan — **✓** §6.
- F13 (Full) Zoom 1×–10× — **✓** §6.
- F14 (Full) Grid expansion ghost — **✓** §12.

### G. Characters — FSM & AI

- G1-G7 — **✓** §5 (client-side FSM; deterministic from `worldSeed`).

### H. Asset System

- H1-H13 — **✓** §13 (lazy tier rendering — Addresses MAJ-8).

### I. Layout Editor

- I1-I15 — **✓** §12.

### J. Seats

- J1-J4 — **✓** §13, §10 (`agent.reassignSeat`).

### K. Persistence

- K1-K9 — **✓** §16 (writer-tag — Addresses MAJ-11).

### L. Multi-Window / Multi-Client

- L1-L4 — **✓** §4, §16.

### M. Sound & Notifications

- M1-M3 — **✓** §15 (PipeWire cascade + notify-send — Addresses MAJ-7).

### N. UI Chrome

- N1 — **✓** §6.
- N2 — **✓** §10, §16.
- N3 — **⚠** §6 "Changelog GIFs and body font" subsection (Addresses NEW-MIN-7): animated in T1/T2-iTerm2; first-frame in T2-WezTerm/T3/T4–T6 with a "(animation; open in browser)" footer.
- N4 — **✓** §6.
- N5 — **✓** §10 (`daemon.log` topic).
- N6 — **⚠** §6 "Changelog GIFs and body font" subsection (Addresses NEW-MIN-7): sprite-rendered headings/toolbars/action bar in image tiers + Sixel; body text always uses the terminal's own font (no protocol exists to override); T4 attempts half-block heading rendering; T5/T6 fall back to ANSI bold + color.

### O. Distribution & Install

- O1 — **✓** §17 (npm + cargo. Homebrew is post-MVP — Addresses NEW-MIN-6; MVP install path is `npm install -g pixel-agents`).
- O2 — **✓** §4.
- O3 — **✓** §4 + supervisor (CRIT-2).
- O4 — **✓** §17 (opt-in only, no in-place — Addresses MAJ-10).

### P. Testing

- P1-P5 — **✓** §18.

### Q. Cross-Platform

- Q1-Q4 — **✓** §20.

### R. Performance

- R1 (MVP) ≤16 ms in T1 — **✓** §19.
- R2 (MVP) ≤33 ms in fallback — **✓ in T3-foot/wez/mlterm and T4/T5/T6; ⚠ in T3-xterm (15 fps)** — Addresses MAJ-5. Document target as tier-specific.
- R3 (Full) **<200 MB combined** (revised from <100 MB) — **⚠** §19, Addresses MAJ-8. Original target documented as over-promised; revised target met.
- R4 (Full) <5% idle CPU 5 agents — **✓** §19.

### S. Roadmap

- S1-S4 — **✓ non-blocked**: rendering, snapshot/event schema, and provider interfaces are all open-ended.

**Summary**: 0 ✗. ⚠ items: F6 (half-block only), N3 (animated GIFs), N6 (font), R2 (T3-xterm only), R3 (revised target). **Every MVP item is ✓.**

---

## 22. Open Questions

(Pared down from v1; addressed items deleted.)

1. **Snapshot vs delta balance**: at what tick rate does sparse delta beat full snapshot in steady state? With client-side FSM (CRIT-3 resolution) we emit only events, so the question is now "how often do we need to emit `world.snapshot` re-sync to handle dropped clients?" Default: on layout change + every 5 min. Will tune from production telemetry.
2. **Sixel performance on Windows Terminal**: WT 1.22+ added Sixel but throughput is slow; we target 20 fps and ⚠. Acceptable?
3. **node-pty Windows ConPTY edge cases**: claude on Windows occasionally emits sequences ConPTY mishandles; we may need a stripping shim. Telemetry will reveal.
4. **Color management for HSBC across tiers**: Colorize mode produces colors not in xterm-256. In T5/T6 quantization is destructive. We store the full HSBC and quantize at draw (fidelity over memory).

(MIN-1 / OQ-2 promoted to blocking and addressed in §7 + MAJ-4. Migration UX, audio cross-platform, hot-reload-during-edit, future protocol versioning are all resolved in §16 / §15 / §16 / §10 respectively.)

---

## Sources

Primary sources only (Addresses MIN-8):

- [Ratatui — GitHub](https://github.com/ratatui/ratatui)
- [crossterm — crates.io](https://crates.io/crates/crossterm)
- [wezterm-term — embeddable terminal core (README)](https://github.com/wezterm/wezterm/blob/main/term/README.md)
- [alacritty_terminal — docs.rs/0.25](https://docs.rs/alacritty_terminal/0.25.0/alacritty_terminal/)
- [Kitty Graphics Protocol — specification](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
- [libghostty exposes Kitty graphics protocol — Mitchell Hashimoto, Feb 2026](https://x.com/mitchellh/status/2041253090205249584)
- [Ghostty supports unicode placeholders (Kitty protocol) — Mitchell Hashimoto](https://x.com/mitchellh/status/1818696111999299976)
- [Ghostty unicode placeholders thread — Hachyderm](https://hachyderm.io/@mitchellh/112882200482778154)
- [Ghostty Features documentation](https://ghostty.org/docs/features)
- [Ratatui 0.30 highlights — modular workspace release](https://ratatui.rs/highlights/v030/)
- [`ratatui-crossterm` 0.1 — crates.io](https://crates.io/crates/ratatui-crossterm)
- [`tachyonfx` releases — junkdog/tachyonfx](https://github.com/junkdog/tachyonfx/releases)
- [node-pty — Microsoft (releases)](https://github.com/microsoft/node-pty/releases)
- [PipeWire — pipewire.org](https://pipewire.org/)
- [PipeWire/Examples — ArchWiki](https://wiki.archlinux.org/title/PipeWire/Examples)
- [Terminal Compatibility Matrix — tmuxai.dev](https://tmuxai.dev/terminal-compatibility/)
- [`systemd.service` — systemd.io](https://systemd.io/SERVICE_FILE/)
- [`launchd.plist` — Apple developer documentation](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingLaunchdJobs.html)
- [Scheduled Task XML schema — Microsoft Docs](https://learn.microsoft.com/en-us/windows/win32/taskschd/task-scheduler-schema)

---
