# TODO

Active items by priority. For background and big-picture plans, see the linked docs at the bottom.

## Now — Phase 1 Day 9-10

- [ ] **Day 9-10** — daemon-side `AgentEventSink` bus (broadcast over UDS, backpressure).
      Per-agent scope (`emitTo(agentId, ...)`) on top of the Day 7-8 broadcast.
      Socket high-water-mark backpressure pauses PTY pumps.

## Next — Phase 1 Day 11-16

- [ ] **Day 11** — NDJSON logging to `~/.pixel-agents/logs/`.
- [ ] **Day 12** — hook integration test (real `claude` → hook script → daemon → mock client).
- [ ] **Day 13-14** — agent spawn + JSONL polling end-to-end.
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
