import { MethodRegistry } from '../dispatch.js';
import { registerAgentMethods } from './agents.js';
import { registerControlMethods } from './control.js';
import { registerLayoutMethods } from './layout.js';
import { registerSettingsMethods } from './settings.js';
import { registerSubscribeMethod } from './subscribe.js';

/**
 * Build the daemon's complete method catalog. Single call site so server.ts
 * can construct it once at boot and pass it to every connection.
 */
export function buildMethodRegistry(): MethodRegistry {
  const reg = new MethodRegistry();
  registerLayoutMethods(reg);
  registerSettingsMethods(reg);
  registerSubscribeMethod(reg);
  registerControlMethods(reg);
  registerAgentMethods(reg);
  return reg;
}
