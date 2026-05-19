#!/usr/bin/env node
import * as crypto from 'crypto';
import * as fs from 'fs';
import * as net from 'net';

import {
  clearDiscoveryIfOwned,
  type DaemonDiscovery,
  ensurePixelAgentsDir,
  isProcessAlive,
  readDiscovery,
  writeDiscovery,
} from './discovery.js';
import { DAEMON_SOCKET_PATH } from './paths.js';

const DAEMON_VERSION = '0.0.1';
const BOOT_TIMEOUT_MS = 3000;

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

  const state = checkExistingDaemon();
  if (state === 'owned-by-live-pid') {
    const existing = readDiscovery();
    console.error(
      `[Daemon] Another daemon is already running (pid=${existing?.pid}, bootId=${existing?.bootId}). Refusing to start.`,
    );
    process.exit(1);
  }
  if (state === 'stale') {
    console.log('[Daemon] Found stale daemon.json — overwriting.');
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
    console.warn(`[Daemon] Boot took longer than ${BOOT_TIMEOUT_MS}ms`);
  }

  writeDiscovery(discovery);

  // For now: accept connections, immediately close them. Real RPC framing
  // lands in Phase 1 Day 3-4 (P1.5).
  server.on('connection', (sock) => {
    sock.end('daemon-stub\n');
  });

  console.log(
    `[Daemon] Started pid=${process.pid} bootId=${discovery.bootId.slice(0, 8)} socket=${DAEMON_SOCKET_PATH}`,
  );

  let shuttingDown = false;
  const shutdown = (signal: string) => {
    if (shuttingDown) return;
    shuttingDown = true;
    console.log(`[Daemon] Received ${signal}, shutting down.`);
    server.close(() => {
      clearDiscoveryIfOwned(process.pid);
      try {
        if (fs.existsSync(DAEMON_SOCKET_PATH)) fs.unlinkSync(DAEMON_SOCKET_PATH);
      } catch {
        // best effort
      }
      process.exit(0);
    });
    // Hard-kill if close() hangs (orphaned client connections, etc.)
    setTimeout(() => process.exit(0), 2000).unref();
  };

  process.on('SIGTERM', () => shutdown('SIGTERM'));
  process.on('SIGINT', () => shutdown('SIGINT'));

  if (!opts.foreground) {
    // Detach stdin so background launchers don't keep stdin open.
    process.stdin.unref?.();
  }
}

main().catch((err) => {
  console.error('[Daemon] Fatal:', err);
  process.exit(1);
});
