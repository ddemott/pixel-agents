# RESOLVED

> Historical log of completed items pulled out of TODO.md and other working lists. Append-only; grouped by date.

## 2026-05-27

- Answerback / device control routing (Phase 4 PTY fidelity) — `ReplyCollector` + `PtyTerminal::drain_replies()` implemented and wired into the main loop (after PtyOut + tick backstop). Device control replies (DA1, DSR, etc.) are now captured and forwarded via `pty.input`. Also cleaned up 9 pre-existing client clippy lints. (feat/pty-answerback-routing)
