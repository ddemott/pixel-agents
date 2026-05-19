import { err, type Handler, type MethodRegistry, ok } from '../dispatch.js';

/**
 * agent.list returns the persisted registry for a given cwd. Live PTY-backed
 * agents come online once Day 13-14 wires node-pty; until then this is the
 * source of truth for revival.
 */
interface ListParams {
  cwd?: string;
}

function isListParams(p: unknown): p is ListParams {
  if (p === null || p === undefined) return true;
  if (typeof p !== 'object' || Array.isArray(p)) return false;
  const cwd = (p as ListParams).cwd;
  return cwd === undefined || typeof cwd === 'string';
}

/**
 * Methods whose real implementation depends on infrastructure that doesn't
 * exist yet (node-pty PTY hosting, asset loader port). We register them now
 * so clients get a descriptive error code rather than the generic
 * `unknown_method`, and so the catalog is enumerable for future Phase work.
 */
const NOT_YET: Record<string, string> = {
  'agent.spawn': 'agent spawn lands in Phase 1 Day 13-14 (node-pty hosting)',
  'agent.close': 'agent close lands in Phase 1 Day 13-14',
  'agent.focus': 'agent focus lands in Phase 2 Day 6 (focus arbitration)',
  'agent.reassignSeat': 'seat reassignment lands once spawn ships',
  'agent.adopt': 'external session adoption lands once spawn ships',
  'pty.input': 'PTY hosting lands in Phase 1 Day 13-14',
  'pty.resize': 'PTY hosting lands in Phase 1 Day 13-14',
  'pty.resync': 'PTY hosting lands in Phase 1 Day 13-14',
  'assets.list': 'asset loader port lands in Phase 1 Day 6+ (after persistence)',
  'assets.requestBlob': 'asset blob streaming lands in Phase 1 Day 9-10',
  'assets.addDir': 'asset directory management lands once asset loader ships',
  'assets.removeDir': 'asset directory management lands once asset loader ships',
  'hooks.toggle': 'hook toggle RPC lands once persistence covers hook settings',
};

function notYetHandler(method: string): Handler {
  const reason = NOT_YET[method] ?? `not yet supported: ${method}`;
  return () => err('not_yet_supported', reason);
}

export function registerAgentMethods(reg: MethodRegistry): void {
  reg.register('agent.list', (params, _s, ctx) => {
    if (!isListParams(params)) {
      return err('bad_params', 'agent.list expects { cwd?: string } or no params');
    }
    const cwd = params?.cwd ?? process.cwd();
    return ok({ agents: ctx.agents.forCwd(cwd) });
  });

  for (const method of Object.keys(NOT_YET)) {
    reg.register(method, notYetHandler(method));
  }
}
