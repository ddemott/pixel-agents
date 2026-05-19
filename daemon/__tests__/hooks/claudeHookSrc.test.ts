import { spawn } from 'child_process';
import * as fs from 'fs';
import * as http from 'http';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

const HOOK_SCRIPT = path.join(__dirname, '../../../dist/hooks/claude-hook.js');

// Isolated temp HOME
let tmpBase: string;

function writeServerJson(port: number, token: string): void {
  const dir = path.join(tmpBase, '.pixel-agents');
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(
    path.join(dir, 'server.json'),
    JSON.stringify({ port, pid: process.pid, token, startedAt: Date.now() }),
  );
}

function writeDaemonJson(hookPort: number, token: string): void {
  const dir = path.join(tmpBase, '.pixel-agents');
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(
    path.join(dir, 'daemon.json'),
    JSON.stringify({
      bootId: '00000000-0000-0000-0000-000000000000',
      token,
      pid: process.pid,
      socketPath: path.join(dir, 'daemon.sock'),
      hookPort,
      startedAt: Date.now(),
      version: '0.0.1',
    }),
  );
}

/**
 * Spin up an HTTP server that captures POSTed body + Authorization header,
 * then runs the hook script with provided HOME + env overrides.
 */
async function withReceiver<T>(
  fn: (info: {
    port: number;
    received: Array<{ body: string; authorization: string | undefined }>;
  }) => Promise<T>,
): Promise<T> {
  const received: Array<{ body: string; authorization: string | undefined }> = [];
  const server = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c: Buffer) => (body += c.toString()));
    req.on('end', () => {
      received.push({
        body,
        authorization: (req.headers.authorization as string | undefined) ?? undefined,
      });
      res.writeHead(200);
      res.end('ok');
    });
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  try {
    const port = (server.address() as { port: number }).port;
    return await fn({ port, received });
  } finally {
    server.close();
  }
}

/** Run the hook script with given stdin + env overrides; returns exit code. */
function runHookScript(
  stdin: string,
  envExtra: Record<string, string | undefined> = {},
): Promise<{ code: number | null; stdout: string }> {
  return new Promise((resolve) => {
    const env: NodeJS.ProcessEnv = { ...process.env, HOME: tmpBase };
    // Always strip the discovery envs from the parent before applying overrides
    // so a leaked var from the parent shell can't poison the test.
    delete env.PIXEL_AGENTS_HOOK_URL;
    delete env.PIXEL_AGENTS_HOOK_TOKEN;
    for (const [key, value] of Object.entries(envExtra)) {
      if (value === undefined) {
        delete env[key];
      } else {
        env[key] = value;
      }
    }
    const child = spawn('node', [HOOK_SCRIPT], {
      env,
      stdio: ['pipe', 'pipe', 'pipe'],
      timeout: 5000,
    });
    let stdout = '';
    child.stdout.on('data', (d: Buffer) => (stdout += d.toString()));
    child.on('close', (code) => resolve({ code, stdout }));
    child.stdin.write(stdin);
    child.stdin.end();
  });
}

describe('claude-hook.js integration', () => {
  beforeEach(() => {
    tmpBase = fs.mkdtempSync(path.join(os.tmpdir(), 'pxl-hook-int-'));
  });

  afterEach(() => {
    try {
      fs.rmSync(tmpBase, { recursive: true, force: true });
    } catch {
      /* ignore */
    }
  });

  // Skip if hook script not built
  function skipIfNotBuilt(): void {
    if (!fs.existsSync(HOOK_SCRIPT)) {
      console.warn(`Skipping: ${HOOK_SCRIPT} not found. Run 'npm run compile' first.`);
    }
  }

  // 1. Script reads stdin and POSTs to server
  it('reads stdin and POSTs to server', async () => {
    skipIfNotBuilt();
    if (!fs.existsSync(HOOK_SCRIPT)) return;

    const received: string[] = [];
    const server = http.createServer((req, res) => {
      let body = '';
      req.on('data', (c: Buffer) => (body += c.toString()));
      req.on('end', () => {
        received.push(body);
        res.writeHead(200);
        res.end('ok');
      });
    });

    await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
    const port = (server.address() as { port: number }).port;
    writeServerJson(port, 'test-token');

    const event = JSON.stringify({ session_id: 'abc', hook_event_name: 'Stop' });
    const { code } = await runHookScript(event);

    server.close();
    expect(code).toBe(0);
    expect(received).toHaveLength(1);
    expect(JSON.parse(received[0]).session_id).toBe('abc');
  });

  // 2. Script exits 0 on missing server.json
  it('exits 0 when server.json is missing', async () => {
    skipIfNotBuilt();
    if (!fs.existsSync(HOOK_SCRIPT)) return;

    // Don't write server.json
    const { code } = await runHookScript(
      JSON.stringify({ session_id: 'x', hook_event_name: 'Stop' }),
    );
    expect(code).toBe(0);
  });

  // 5. Script exits 0 on invalid stdin
  it('exits 0 on invalid stdin', async () => {
    skipIfNotBuilt();
    if (!fs.existsSync(HOOK_SCRIPT)) return;

    writeServerJson(9999, 'tok');
    const { code } = await runHookScript('not json at all!!!');
    expect(code).toBe(0);
  });

  // 6. Script handles server timeout
  it('exits within 5s when server does not respond', async () => {
    skipIfNotBuilt();
    if (!fs.existsSync(HOOK_SCRIPT)) return;

    // Start a server that never responds
    const server = http.createServer(() => {
      // intentionally never respond
    });
    await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
    writeServerJson((server.address() as { port: number }).port, 'tok');

    const start = Date.now();
    const { code } = await runHookScript(
      JSON.stringify({ session_id: 'x', hook_event_name: 'Stop' }),
    );
    const elapsed = Date.now() - start;

    server.close();
    expect(code).toBe(0);
    expect(elapsed).toBeLessThan(5000);
  });

  // ── Discovery chain: env override → daemon.json → server.json ──────

  // 7. daemon.json with hookPort wins over server.json
  it('prefers daemon.json over server.json when both are present', async () => {
    skipIfNotBuilt();
    if (!fs.existsSync(HOOK_SCRIPT)) return;

    await withReceiver(async ({ port: daemonPort, received: daemonReceived }) => {
      await withReceiver(async ({ port: legacyPort, received: legacyReceived }) => {
        writeDaemonJson(daemonPort, 'daemon-tok');
        writeServerJson(legacyPort, 'legacy-tok');

        const { code } = await runHookScript(
          JSON.stringify({ session_id: 'abc', hook_event_name: 'Stop' }),
        );

        expect(code).toBe(0);
        expect(daemonReceived).toHaveLength(1);
        expect(legacyReceived).toHaveLength(0);
        expect(daemonReceived[0].authorization).toBe('Bearer daemon-tok');
      });
    });
  });

  // 8. daemon.json without hookPort falls through to server.json
  it('falls back to server.json when daemon.json has no hookPort', async () => {
    skipIfNotBuilt();
    if (!fs.existsSync(HOOK_SCRIPT)) return;

    await withReceiver(async ({ port: legacyPort, received: legacyReceived }) => {
      // Write daemon.json WITHOUT hookPort (the Day-1/2 boot shape)
      const dir = path.join(tmpBase, '.pixel-agents');
      fs.mkdirSync(dir, { recursive: true });
      fs.writeFileSync(
        path.join(dir, 'daemon.json'),
        JSON.stringify({
          bootId: '00000000-0000-0000-0000-000000000000',
          token: 'no-hookport',
          pid: process.pid,
          socketPath: path.join(dir, 'daemon.sock'),
          startedAt: Date.now(),
          version: '0.0.1',
        }),
      );
      writeServerJson(legacyPort, 'legacy-tok');

      const { code } = await runHookScript(
        JSON.stringify({ session_id: 'abc', hook_event_name: 'Stop' }),
      );

      expect(code).toBe(0);
      expect(legacyReceived).toHaveLength(1);
      expect(legacyReceived[0].authorization).toBe('Bearer legacy-tok');
    });
  });

  // 9. PIXEL_AGENTS_HOOK_URL env override beats both discovery files
  it('env override wins over daemon.json and server.json', async () => {
    skipIfNotBuilt();
    if (!fs.existsSync(HOOK_SCRIPT)) return;

    await withReceiver(async ({ port: envPort, received: envReceived }) => {
      await withReceiver(async ({ port: daemonPort, received: daemonReceived }) => {
        writeDaemonJson(daemonPort, 'daemon-tok');

        const { code } = await runHookScript(
          JSON.stringify({ session_id: 'abc', hook_event_name: 'Stop' }),
          { PIXEL_AGENTS_HOOK_URL: `http://127.0.0.1:${envPort}` },
        );

        expect(code).toBe(0);
        expect(envReceived).toHaveLength(1);
        expect(daemonReceived).toHaveLength(0);
        // No PIXEL_AGENTS_HOOK_TOKEN set → no Authorization header.
        expect(envReceived[0].authorization).toBeUndefined();
      });
    });
  });

  // 10. PIXEL_AGENTS_HOOK_TOKEN attaches Bearer when env URL is used
  it('attaches PIXEL_AGENTS_HOOK_TOKEN to env-URL requests', async () => {
    skipIfNotBuilt();
    if (!fs.existsSync(HOOK_SCRIPT)) return;

    await withReceiver(async ({ port: envPort, received: envReceived }) => {
      const { code } = await runHookScript(
        JSON.stringify({ session_id: 'abc', hook_event_name: 'Stop' }),
        {
          PIXEL_AGENTS_HOOK_URL: `http://127.0.0.1:${envPort}`,
          PIXEL_AGENTS_HOOK_TOKEN: 'env-token',
        },
      );

      expect(code).toBe(0);
      expect(envReceived).toHaveLength(1);
      expect(envReceived[0].authorization).toBe('Bearer env-token');
    });
  });

  // 11. Malformed env URL silently falls through to discovery files
  it('falls through to server.json when PIXEL_AGENTS_HOOK_URL is malformed', async () => {
    skipIfNotBuilt();
    if (!fs.existsSync(HOOK_SCRIPT)) return;

    await withReceiver(async ({ port: legacyPort, received: legacyReceived }) => {
      writeServerJson(legacyPort, 'legacy-tok');

      const { code } = await runHookScript(
        JSON.stringify({ session_id: 'abc', hook_event_name: 'Stop' }),
        { PIXEL_AGENTS_HOOK_URL: 'not-a-url' },
      );

      expect(code).toBe(0);
      expect(legacyReceived).toHaveLength(1);
    });
  });
});
