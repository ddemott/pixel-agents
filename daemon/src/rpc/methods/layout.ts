import { type Layout, writeLayout } from '../../layout/persistence.js';
import type { MethodRegistry } from '../dispatch.js';
import { err, ok } from '../dispatch.js';

interface SaveParams {
  layout: Layout;
}

interface ImportParams {
  layout: Layout;
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function isSaveParams(p: unknown): p is SaveParams {
  return isObject(p) && isObject((p as Record<string, unknown>).layout);
}

function isImportParams(p: unknown): p is ImportParams {
  return isSaveParams(p);
}

export function registerLayoutMethods(reg: MethodRegistry): void {
  reg.register('layout.get', (_p, _s, ctx) => {
    return ok({ layout: ctx.state.layout });
  });

  reg.register('layout.save', (params, _s, ctx) => {
    if (!isSaveParams(params)) {
      return err('bad_params', 'layout.save requires { layout: object }');
    }
    // Debounced write — multiple rapid saves coalesce, matching the K5 spec.
    ctx.layoutDebouncer.schedule(params.layout);
    // Update the in-memory layout immediately so subsequent layout.get reads
    // observe the new state (the file write itself is debounced).
    ctx.state.layout = params.layout;
    // Broadcast to everyone — including the originating client, which is fine
    // because clients dedupe via their local writerTag of the change.
    ctx.sink.post({ type: 'layout.changed', source: 'client', layout: params.layout });
    return ok({});
  });

  reg.register('layout.import', (params, _s, ctx) => {
    if (!isImportParams(params)) {
      return err('bad_params', 'layout.import requires { layout: object }');
    }
    // Imports are a deliberate user action — write immediately, no debounce.
    writeLayout(params.layout, ctx.ours);
    ctx.state.layout = params.layout;
    ctx.sink.post({ type: 'layout.changed', source: 'client', layout: params.layout });
    return ok({});
  });

  reg.register('layout.export', (_p, _s, ctx) => {
    if (ctx.state.layout === null) {
      return err('no_layout', 'no layout loaded to export');
    }
    return ok({ layout: ctx.state.layout });
  });

  // K9: writing the bundled default to webview-ui/public/assets/default-layout.json
  // belongs to the VS Code extension's command. Phase 6+ will move it here; for
  // now the daemon doesn't ship it.
  reg.register('layout.setDefault', () =>
    err('not_yet_supported', 'layout.setDefault lands in Phase 6 (bundled-default authoring)'),
  );
}
