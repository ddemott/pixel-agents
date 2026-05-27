# Pixel Agents — Compressed Reference

VS Code extension with embedded React webview: pixel art office where AI agents (Claude Code terminals) are animated characters.

## Architecture

Top-level layout (full per-file tree + subsystem internals in **`ARCHITECTURE.md`**):

- `src/` — Extension backend (VS Code API; CommonJS via esbuild). Terminal lifecycle, JSONL watching, asset/layout/config persistence, hook server integration. Decoupled from vscode via AgentEventSink/TerminalRegistry/AgentRuntime/AgentStateStore (TUI port Phase 0).
- `daemon/` — Standalone daemon (TUI port Phase 1; ESM, Node 22). UDS RPC (channel-mux framing, NDJSON method registry), BroadcastSink fan-out, writer-tagged file persistence, ported hook HTTP server (`daemon/src/hooks/`, a CJS subtree).
- `webview-ui/src/` — React + TypeScript (Vite). Imperative `OfficeState` game engine, canvas renderer, layout editor, sprite/asset pipeline.
- `client/` — Rust TUI client (TUI port Phase 2+; Cargo workspace). Capability-tiered renderer (Kitty/iTerm2/Sixel/cell), scene compositor, PTY hosting (`pty/mod.rs`), UDS daemon connection.
- `scripts/` — 7-stage asset extraction pipeline (detect → metadata → export PNGs + furniture-catalog.json).

**See `ARCHITECTURE.md`** for: the exhaustive file tree, Agent Status Tracking internals, Office UI, Layout Editor, and Asset System details.

## Core Concepts

**Vocabulary**: Terminal = VS Code terminal running Claude. Session = JSONL conversation file. Agent = webview character bound 1:1 to a terminal.

**Extension ↔ Webview**: `postMessage` protocol. Key messages: `openClaude`, `agentCreated/Closed`, `focusAgent`, `agentToolStart/Done/Clear`, `agentStatus`, `existingAgents`, `layoutLoaded`, `furnitureAssetsLoaded`, `floorTilesLoaded`, `wallTilesLoaded`, `saveLayout`, `saveAgentSeats`, `exportLayout`, `importLayout`, `settingsLoaded` (includes `externalAssetDirectories`), `setSoundEnabled`, `addExternalAssetDirectory`, `removeExternalAssetDirectory` (field: `path`), `externalAssetDirectoriesUpdated` (field: `dirs`).

**One-agent-per-terminal**: Each "+ Agent" click → new terminal (`claude --session-id <uuid>`) → immediate agent creation → 1s poll for `<uuid>.jsonl` → file watching starts.

**Terminal adoption**: Project-level 1s scan detects unknown JSONL files. If active terminal has no agent → adopt. If focused agent exists → reassign (`/clear` handling).

> **Agent Status Tracking** (JSONL records, dual-mode hooks/heuristic detection, hook discovery chain, persistence), **Office UI** (rendering, characters, sub-agents, bubbles, seats), **Layout Editor**, and **Asset System** are documented in `ARCHITECTURE.md`.

## Condensed Lessons

- `fs.watch` unreliable on Windows — always pair with polling backup
- Partial line buffering essential for append-only file reads (carry unterminated lines)
- Delay `agentToolDone` 300ms to prevent React batching from hiding brief active states
- **Idle detection** has two signals: (1) `system` + `subtype: "turn_duration"` — reliable for tool-using turns (~98%), emitted once per completed turn, handler clears all tool state as safety measure. (2) Text-idle timer (`TEXT_IDLE_DELAY_MS = 5s`) — for text-only turns where `turn_duration` is never emitted. Only starts when `hadToolsInTurn` is false (no tools used yet in this turn); if any tool_use arrives, `hadToolsInTurn` becomes true and the timer is suppressed for the rest of the turn. Reset on new user prompt or `turn_duration`. Cancelled by ANY new JSONL data arriving in `readNewLines`. Only fires after 5s of complete file silence
- User prompt `content` can be string (text) or array (tool_results) — handle both
- `/clear` creates NEW JSONL file (old file just stops)
- `--output-format stream-json` needs non-TTY stdin — can't use with VS Code terminals
- Hook-based IPC failed (hooks captured at startup, env vars don't propagate). JSONL watching works
- PNG→SpriteData: pngjs for RGBA buffer, alpha threshold 2 (`PNG_ALPHA_THRESHOLD`), supports `#RRGGBBAA` semi-transparent pixels
- OfficeCanvas selection changes are imperative (`editorState.selectedFurnitureUid`); must call `onEditorSelectionChange()` to trigger React re-render for toolbar

## Build & Dev

```sh
npm install && cd webview-ui && npm install && cd ../daemon && npm install && cd .. && npm run build
```

Build: type-check → lint → esbuild (extension + hook bundle) → vite (webview). F5 for Extension Dev Host.

Daemon build (TUI port Phase 1): `cd daemon && npm run build` produces `daemon/dist/daemon/src/server.js` (post-build step also writes `dist/src/package.json {"type":"commonjs"}` so Node 22 ESM can interop with the cross-imported CJS Phase-0 modules under `dist/src/`). Run with `node daemon/dist/daemon/src/server.js --foreground` or `npm start` from `daemon/`.

Testing:

- `npm test` -- all unit/integration tests (webview + daemon)
- `npm run test:daemon` -- daemon tests (Vitest)
- `npm run test:webview` -- webview asset integration tests (Node test runner)
- `npm run e2e` -- Playwright E2E tests (real VS Code instance)

## Recommended Development Workflow

**Always** follow the disciplined lifecycle:

1. Start every piece of work with `/start-feature "short description"`.
   - This creates the branch, runs baseline gates, and adds tracking to `TODO.md`.

2. While coding, run targeted gates frequently:
   - `npm run lint && npm run check-types`
   - Relevant tests only: `npm run test:daemon`, `npm run test:webview`, or `npm test`
   - Selective E2E when touching user-visible or PTY/focus paths: `npm run e2e` (or a specific test)

3. Every new or changed test **must** contain 5W diagnostic comments (Who / What / When / Where / Why).

4. Before finalizing, run `/review --local` (self-review) and update all affected `*.md` files (CLAUDE.md, ARCHITECTURE.md, TODO.md, CONTRIBUTING.md, docs/, etc.).

5. When the slice or feature is ready, run `/commit-code`.
   - It enforces lint, build, relevant tests, documentation updates, RESOLVED.md hygiene, conventional commit message, secret scanning, and branch rules.

6. For code review use `/review --pr <number>` (or the GitHub review UI).

See `CONTRIBUTING.md` → "Development Workflow (The Proper Way)" for the full detailed checklist.

## Portable Workflow Layer (adopted 2026)

In addition to the internal slash-command system (`/start-feature`, `/review`, `/commit-code`), the project has adopted the portable development workflow kit:

- `npm run create-branch <type>/<name>` — creates a properly prefixed branch from latest `main`, pulls latest, and runs initial gates.
- `npm run prepare-commit` — runs the full automated quality gate defined in `workflow.config.json` (format check, lint, type checking, full test suite, focused/skipped test scan, staged-file heuristics).

**Key integration decisions**:

- Existing `.husky` hooks were left unchanged (gitleaks secret scanning + lint-staged in pre-commit; `check-types` in pre-push). They provide strong, fast, security-focused gates.
- `workflow.config.json` lives at the project root and is the single source of truth for all gate commands and documentation requirements.
- The portable scripts are a complementary layer. The canonical way to begin work remains `/start-feature`, and final commits still go through the `/commit-code` process.

See `portable-workflow-kit/README.md` and `ADOPTING_THE_WORKFLOW.md` (copied locally during adoption) for details on the kit itself. The only long-term file that needs maintenance is `workflow.config.json`.

Direct commits to `main` and "I'll fix the tests later" commits are not acceptable.

## TypeScript Constraints

- No `enum` (`erasableSyntaxOnly`) — use `as const` objects
- `import type` required for type-only imports (`verbatimModuleSyntax`)
- `noUnusedLocals` / `noUnusedParameters`

## Constants

All magic numbers and strings are centralized — never add inline constants to source files:

- **Extension backend**: `src/constants.ts` — timing intervals, display truncation limits, PNG/asset parsing values, VS Code command/key identifiers
- **Webview**: `webview-ui/src/constants.ts` — grid/layout sizes, character animation speeds, matrix effect params, rendering offsets/colors, camera, zoom, editor defaults, game logic thresholds
- **CSS styling**: `webview-ui/src/index.css` `:root` block — `--pixel-*` custom properties for UI colors, backgrounds, borders, z-indices used in React inline styles
- **Canvas overlay colors** (rgba strings for seats, grids, ghosts, buttons) live in the webview constants file since they're used in canvas 2D context, not CSS
- `webview-ui/src/office/types.ts` re-exports grid/layout constants (`TILE_SIZE`, `DEFAULT_COLS`, etc.) from `constants.ts` for backward compatibility — import from either location

## Key Patterns

- `crypto.randomUUID()` works in VS Code extension host
- Terminal `cwd` option sets working directory at creation
- `/add-dir <path>` grants session access to additional directory

## Windows-MCP (Desktop Automation)

- `uvx --python 3.13 windows-mcp` — Tools: Snapshot, Click, Type, Scroll, Move, Shortcut, App, Shell, Wait, Scrape
- Webview buttons show `(0,0)` in a11y tree — must use `Snapshot(use_vision=true)` for coordinates
- Snap both VS Code windows side-by-side on SAME screen before clicking in Extension Dev Host
- Reload extension via button on main VS Code window after building

## Key Decisions

- `WebviewViewProvider` (not `WebviewPanel`) — lives in panel area alongside terminal
- Inline esbuild problem matcher (no extra extension needed)
- Webview is separate Vite project with own `node_modules`/`tsconfig`
- Hook script (`daemon/src/hooks/providers/hook/claude/hooks/claudeHookSrc.ts`) bundled to standalone CJS via esbuild (`buildHooks()` in esbuild.js); output filename pinned to `dist/hooks/claude-hook.js` (Claude Code references the exact path)
- Constants centralized in `daemon/src/hooks/constants.ts` (shared), `src/constants.ts` imports from there. Extension-only constants stay in `src/constants.ts`
- `daemon/src/hooks/` is a CJS subtree (`package.json: {"type": "commonjs"}`) inside the otherwise-ESM daemon — lets the VS Code extension (CJS) import hooks directly without ESM/CJS interop errors. Daemon's own ESM code uses standard ESM→CJS interop when it eventually imports from hooks/
- Server always starts regardless of hooks toggle (foundation for future WS transport). Only hook installation is gated by the setting
