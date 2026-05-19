import { err, type MethodRegistry, ok } from '../dispatch.js';

interface SubscribeParams {
  topics: string[];
}

function isSubscribeParams(p: unknown): p is SubscribeParams {
  return (
    typeof p === 'object' &&
    p !== null &&
    Array.isArray((p as SubscribeParams).topics) &&
    (p as SubscribeParams).topics.every((t) => typeof t === 'string')
  );
}

/**
 * `subscribe` updates the connection's topic filter. Default at handshake is
 * `*` (no filter). Passing an empty array effectively mutes the client until
 * it subscribes again. Passing `["*"]` re-enables the all-topics default.
 */
export function registerSubscribeMethod(reg: MethodRegistry): void {
  reg.register('subscribe', (params, scope) => {
    if (!isSubscribeParams(params)) {
      return err('bad_params', 'subscribe requires { topics: string[] }');
    }
    scope.subscriptions.clear();
    for (const t of params.topics) scope.subscriptions.add(t);
    return ok({ topics: [...scope.subscriptions] });
  });
}
