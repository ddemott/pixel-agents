#!/usr/bin/env node
import * as crypto from 'crypto';
import * as fs from 'fs';
import * as net from 'net';

import { setAgentRuntime } from '../../src/agentRuntime.js';
import { BroadcastSink } from './agents/broadcastSink.js';
import { DaemonRuntime } from './agents/daemonRuntime.js';
import { FileStateStore } from './agents/fileStateStore.js';
import { LiveAgents } from './agents/liveAgents.js';
import { AgentsRegistry } from './agents/registry.js';
import { reviveAgentsOnBoot } from './agents/resume.js';
import { readConfig, watchConfig } from './config/persistence.js';
import {
  clearDiscoveryIfOwned,
  type DaemonDiscovery,
  ensurePixelAgentsDir,
  isProcessAlive,
  readDiscovery,
  writeDiscovery,
} from './discovery.js';
import { DaemonHookBridge } from './hookHost/bridge.js';
import { type HookHTTPServerHandle, startHookServer } from './hookHost/server.js';
import { LayoutSaveDebouncer, readLayout, watchLayout } from './layout/persistence.js';
import { createFileLogger, type Logger } from './logging/logger.js';
import { sweepLogs } from './logging/retention.js';
import { DAEMON_LOG_DIR, DAEMON_SOCKET_PATH, DAEMON_STATE_PATH } from './paths.js';
import type { WriterTag } from './persistence/writerTag.js';
import { attachConnection } from './rpc/connection.js';
import type { DispatchContext } from './rpc/dispatch.js';
import { buildMethodRegistry } from './rpc/methods/index.js';
import type { WorldSnapshot } from './rpc/wire.js';

const DAEMON_VERSION = '0.0.1';
const BOOT_TIMEOUT_MS = 3000;
const LOG_GZIP_AFTER_DAYS = 7;
const LOG_DELETE_AFTER_DAYS = 30;
const LOG_SWEEP_INTERVAL_MS = 24 * 60 * 60 * 1000;

interface BootOptions {
  foreground: boolean;
}

function parseArgs(argv: string[]): BootOptions {
  return {
    foreground: argv.includes('--foreground') || argv.includes('-f'),
  };
}

function checkExistingDaemon(): 'free' | 'owned-by-live-pid' | 'stale' {
  const existing = readDiscovery();
  if (!existing) return 'free';
  if (isProcessAlive(existing.pid)) return 'owned-by-live-pid';
  return 'stale';
}

async function bindSocket(): Promise<net.Server> {
  // If a stale socket file exists, remove it so net.Server.listen succeeds.
  if (fs.existsSync(DAEMON_SOCKET_PATH)) {
    try {
      fs.unlinkSync(DAEMON_SOCKET_PATH);
    } catch (e) {
      throw new Error(`Failed to remove stale socket at ${DAEMON_SOCKET_PATH}: ${e}`);
    }
  }

  const server = net.createServer();

  await new Promise<void>((resolve, reject) => {
    const onError = (err: Error) => {
      server.removeListener('listening', onListening);
      reject(err);
    };
    const onListening = () => {
      server.removeListener('error', onError);
      resolve();
    };
    server.once('error', onError);
    server.once('listening', onListening);
    server.listen(DAEMON_SOCKET_PATH);
    // Restrict socket permissions to the current user only.
    setTimeout(() => {
      try {
        fs.chmodSync(DAEMON_SOCKET_PATH, 0o600);
      } catch {
        // best effort
      }
    }, 0);
  });

  return server;
}

async function main(): Promise<void> {
  const opts = parseArgs(process.argv.slice(2));
  const startedAt = Date.now();

  ensurePixelAgentsDir();

  const config = readConfig();

  // Boot the file logger before anything else so config-load + discovery
  // events land in the NDJSON log. `--foreground` also mirrors to stderr so
  // operators get live feedback without `tail -f`.
  const logger: Logger = createFileLogger({
    dir: DAEMON_LOG_DIR,
    prefix: 'daemon',
    level: config.logLevel,
    mirror: opts.foreground ? process.stderr : undefined,
  });
  logger.info(
    {
      module: 'boot',
      externalAssetDirectories: config.externalAssetDirectories.length,
      logLevel: config.logLevel,
    },
    'config loaded',
  );

  const state = checkExistingDaemon();
  if (state === 'owned-by-live-pid') {
    const existing = readDiscovery();
    logger.error(
      { module: 'boot', existingPid: existing?.pid, existingBootId: existing?.bootId },
      'another daemon is already running; refusing to start',
    );
    process.exit(1);
  }
  if (state === 'stale') {
    logger.info({ module: 'boot' }, 'found stale daemon.json — overwriting');
  }

  const discovery: DaemonDiscovery = {
    bootId: crypto.randomUUID(),
    token: crypto.randomBytes(32).toString('hex'),
    pid: process.pid,
    socketPath: DAEMON_SOCKET_PATH,
    startedAt,
    version: DAEMON_VERSION,
  };

  // Bind socket before publishing discovery so clients never see a daemon.json
  // whose socket isn't yet accepting connections.
  const bootDeadline = Date.now() + BOOT_TIMEOUT_MS;
  const server = await bindSocket();
  if (Date.now() > bootDeadline) {
    logger.warn({ module: 'boot', timeoutMs: BOOT_TIMEOUT_MS }, 'boot took longer than expected');
  }

  writeDiscovery(discovery);

  const ours: WriterTag = { processId: process.pid, bootId: discovery.bootId };

  // Wire Phase-0 modules + persistence:
  //  - BroadcastSink fans events out to authed clients.
  //  - DaemonRuntime exposes the boot cwd as a workspace folder.
  //  - FileStateStore is the daemon's untyped scratchpad (daemon-state.json).
  //  - AgentsRegistry owns the typed per-cwd agents.json (arch §16).
  //  - Layout + config are read on boot and watched for external writes;
  //    own-writes are filtered via the writer tag.
  // TerminalRegistry stays Null until node-pty hosting lands in Day 13-14.
  const sink = new BroadcastSink();
  sink.setLogger(logger);
  const stateStore = new FileStateStore(DAEMON_STATE_PATH);
  const agents = new AgentsRegistry(ours);
  setAgentRuntime(new DaemonRuntime(process.cwd()));
  void stateStore;
  void agents;

  // Hook HTTP server lives on 127.0.0.1 only and reuses the UDS auth token.
  // The bridge maps incoming hook events to agent.* topics on the sink so a
  // mock client subscribed to `agent:*` sees toolStart/toolDone/etc. in real
  // time. Boot order: sink → bridge → http server → republish daemon.json
  // with the bound port so hook scripts can find us.
  const hookBridge = new DaemonHookBridge(sink, logger);
  const hookHandle: HookHTTPServerHandle = await startHookServer({
    token: discovery.token,
    onEvent: (providerId, event) => hookBridge.handleEvent(providerId, event),
    logger,
  });
  discovery.hookPort = hookHandle.port;
  writeDiscovery(discovery);

  const sharedState: DispatchContext['state'] = {
    layout: readLayout(),
    config: readConfig(),
  };

  const layoutWatcher = watchLayout(ours, (next) => {
    sharedState.layout = next;
    sink.post({ type: 'layout.changed', source: 'file', layout: next });
  });

  const configWatcher = watchConfig(ours, (next) => {
    sharedState.config = next;
    if (next.logLevel !== sharedState.config.logLevel) {
      logger.info(
        { module: 'config', logLevel: next.logLevel },
        'log level updated from external write',
      );
    }
    logger.setLevel(next.logLevel);
    sink.post({ type: 'settings.updated', settings: next });
  });

  // Retention sweep: gz @ 7d, delete @ 30d. Run once at boot, then daily.
  const runSweep = (): void => {
    const result = sweepLogs({
      dir: DAEMON_LOG_DIR,
      gzipAfterDays: LOG_GZIP_AFTER_DAYS,
      deleteAfterDays: LOG_DELETE_AFTER_DAYS,
    });
    if (result.gzipped.length || result.deleted.length || result.errors.length) {
      logger.info(
        {
          module: 'logRetention',
          gzipped: result.gzipped.length,
          deleted: result.deleted.length,
          errors: result.errors.length,
        },
        'log retention sweep complete',
      );
    }
  };
  runSweep();
  const sweepTimer = setInterval(runSweep, LOG_SWEEP_INTERVAL_MS);
  sweepTimer.unref();

  const layoutDebouncer = new LayoutSaveDebouncer(ours);
  const registry = buildMethodRegistry();
  const liveAgents = new LiveAgents();

  // Forward-declared so `daemon.shutdown` (registered before this assignment
  // resolves) can call into the shutdown handler defined below.
  let shutdown: (signal: string) => void = () => {};
  const dispatchContext: DispatchContext = {
    ours,
    sink,
    agents,
    layoutDebouncer,
    liveAgents,
    hookBridge,
    logger,
    state: sharedState,
    triggerShutdown: () => shutdown('rpc'),
  };

  const buildWorldSnapshot = (): WorldSnapshot => ({
    schemaVersion: 1,
    worldSeed: 0,
    layout: sharedState.layout,
    assets: { catalog: [], characters: [], floors: [], walls: [] },
    agents: [],
  });

  server.on('connection', (sock) => {
    attachConnection(sock, {
      expectedToken: discovery.token,
      bootId: discovery.bootId,
      daemonVersion: DAEMON_VERSION,
      buildWorldSnapshot,
      registry,
      dispatchContext,
      onAuthenticated: (authed, scope) => {
        sink.register(authed, scope.subscriptions);
      },
    });
  });

  logger.info(
    {
      module: 'boot',
      pid: process.pid,
      bootId: discovery.bootId,
      socket: DAEMON_SOCKET_PATH,
      hookPort: hookHandle.port,
    },
    'daemon started',
  );

  // Revive persisted agents in the background. Clients that connect during
  // revival will receive `agent.created { isResumed: true }` events as each
  // agent passes its 3 s health check.
  void reviveAgentsOnBoot({ agents, liveAgents, sink, hookBridge, logger }).catch((e) => {
    logger.error(
      { module: 'resume', error: e instanceof Error ? e.message : String(e) },
      'reviveAgentsOnBoot threw unexpectedly',
    );
  });

  let shuttingDown = false;
  shutdown = (signal: string): void => {
    if (shuttingDown) return;
    shuttingDown = true;
    logger.info({ module: 'shutdown', signal }, 'shutting down');
    clearInterval(sweepTimer);
    layoutDebouncer.flushNow();
    layoutWatcher.dispose();
    configWatcher.dispose();
    // Kill every live PTY so claude children don't outlive us. SIGTERM first;
    // the per-agent KILL_GRACE inside agent.close handles escalation when the
    // shutdown is RPC-triggered. On signal shutdown we lean on the daemon's
    // own 2s hard-kill timer below.
    for (const live of liveAgents.list()) live.pty.kill('SIGTERM');
    void hookHandle.close();
    server.close(() => {
      clearDiscoveryIfOwned(process.pid);
      try {
        if (fs.existsSync(DAEMON_SOCKET_PATH)) fs.unlinkSync(DAEMON_SOCKET_PATH);
      } catch {
        // best effort
      }
      logger.close();
      process.exit(0);
    });
    // Hard-kill if close() hangs (orphaned client connections, etc.)
    setTimeout(() => {
      logger.close();
      process.exit(0);
    }, 2000).unref();
  };

  process.on('SIGTERM', () => shutdown('SIGTERM'));
  process.on('SIGINT', () => shutdown('SIGINT'));

  if (!opts.foreground) {
    // Detach stdin so background launchers don't keep stdin open.
    process.stdin.unref?.();
  }
}

main().catch((err) => {
  // Logger may not have been built yet when this throws; fall back to stderr
  // so the supervisor / launcher always sees fatal boot errors.
  console.error('[Daemon] Fatal:', err);
  process.exit(1);
});
