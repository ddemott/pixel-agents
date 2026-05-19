# TODO

Active items by priority. For background and big-picture plans, see the linked docs at the bottom.

## Now — Phase 1 Day 13-14

- [ ] **Day 13-14** — agent spawn + JSONL polling end-to-end.

## Next — Phase 1 Day 15-16

- [ ] **Day 15-16** — `claude --resume` revival on daemon restart.

## Technical debt surfaced during Phase 0-1

- [ ] `daemon/src/hooks/eventHandler.ts:1` — `TODO(Standalone version)` comment references the
      now-deleted `server/src/` path. Retarget at `daemon/src/`.
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
