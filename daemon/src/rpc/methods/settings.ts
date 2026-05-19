import { type PixelAgentsConfig, writeConfig } from '../../config/persistence.js';
import { err, type MethodRegistry, ok } from '../dispatch.js';

interface SetParams {
  patch: Partial<PixelAgentsConfig>;
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function isSetParams(p: unknown): p is SetParams {
  return isObject(p) && isObject((p as Record<string, unknown>).patch);
}

/** Apply a defensive shallow patch — only known fields, with type checks. */
function applyPatch(
  current: PixelAgentsConfig,
  patch: Partial<PixelAgentsConfig>,
): PixelAgentsConfig {
  const next: PixelAgentsConfig = { ...current };
  if (Array.isArray(patch.externalAssetDirectories)) {
    next.externalAssetDirectories = patch.externalAssetDirectories.filter(
      (d): d is string => typeof d === 'string',
    );
  }
  return next;
}

export function registerSettingsMethods(reg: MethodRegistry): void {
  reg.register('settings.get', (_p, _s, ctx) => {
    return ok({ settings: ctx.state.config });
  });

  reg.register('settings.set', (params, _s, ctx) => {
    if (!isSetParams(params)) {
      return err('bad_params', 'settings.set requires { patch: object }');
    }
    const next = applyPatch(ctx.state.config, params.patch);
    writeConfig(next, ctx.ours);
    ctx.state.config = next;
    ctx.sink.post({ type: 'settings.updated', settings: next });
    return ok({});
  });
}
