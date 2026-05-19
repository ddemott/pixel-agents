import { type MethodRegistry, ok } from '../dispatch.js';

/**
 * `daemon.shutdown` lets a client cleanly stop the daemon (Phase 6 multi-window
 * + Phase 7 packaging both want this). We reply ok immediately, then defer the
 * actual shutdown a tick so the response makes it onto the wire before the
 * socket closes.
 */
export function registerControlMethods(reg: MethodRegistry): void {
  reg.register('daemon.shutdown', (_p, _s, ctx) => {
    setImmediate(() => ctx.triggerShutdown());
    return ok({});
  });
}
