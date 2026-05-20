import { describe, expect, it, vi } from 'vitest';

import { type PtyHandle, PtyHost, type SpawnFn } from '../../src/agents/ptyHost.js';
import { createNullLogger } from '../../src/logging/logger.js';

/** Build a fake `node-pty` IPty handle. Returns the handle plus driver hooks
 *  the test can use to fire data/exit synchronously. */
function makeFakeHandle(): {
  handle: PtyHandle;
  fire: { data(s: string | Buffer): void; exit(code: number, signal?: number): void };
  recorded: {
    writes: string[];
    resizes: Array<[number, number]>;
    signals: string[];
    flow: string[];
  };
} {
  const recorded = {
    writes: [] as string[],
    resizes: [] as Array<[number, number]>,
    signals: [] as string[],
    flow: [] as string[],
  };
  let dataCb: ((d: string | Buffer) => void) | null = null;
  let exitCb: ((e: { exitCode: number; signal?: number }) => void) | null = null;
  const handle: PtyHandle = {
    pid: 12345,
    onData(cb) {
      dataCb = cb;
      return { dispose: () => (dataCb = null) };
    },
    onExit(cb) {
      exitCb = cb;
      return { dispose: () => (exitCb = null) };
    },
    write(data) {
      recorded.writes.push(data);
    },
    resize(cols, rows) {
      recorded.resizes.push([cols, rows]);
    },
    kill(signal) {
      recorded.signals.push(signal ?? 'SIGTERM');
    },
    pause() {
      recorded.flow.push('pause');
    },
    resume() {
      recorded.flow.push('resume');
    },
  };
  return {
    handle,
    recorded,
    fire: {
      data: (s) => dataCb?.(s),
      exit: (code, signal) => exitCb?.({ exitCode: code, signal }),
    },
  };
}

function logger() {
  return createNullLogger();
}

describe('PtyHost', () => {
  it('forwards data to onData callback as Buffer', () => {
    const fake = makeFakeHandle();
    const spawn: SpawnFn = () => fake.handle;
    const onData = vi.fn();
    const onExit = vi.fn();
    new PtyHost(
      { agentId: 1, command: 'x', args: [], cwd: '/', logger: logger(), spawn },
      { onData, onExit },
    );
    fake.fire.data('hello');
    fake.fire.data(Buffer.from([0x1b, 0x5b]));
    expect(onData).toHaveBeenCalledTimes(2);
    expect(onData.mock.calls[0][0]).toBeInstanceOf(Buffer);
    expect((onData.mock.calls[0][0] as Buffer).toString('utf-8')).toBe('hello');
    expect((onData.mock.calls[1][0] as Buffer).length).toBe(2);
  });

  it('fires onExit once with the exit code', () => {
    const fake = makeFakeHandle();
    const onExit = vi.fn();
    const host = new PtyHost(
      {
        agentId: 1,
        command: 'x',
        args: [],
        cwd: '/',
        logger: logger(),
        spawn: () => fake.handle,
      },
      { onData: () => {}, onExit },
    );
    expect(host.isAlive()).toBe(true);
    fake.fire.exit(0);
    expect(onExit).toHaveBeenCalledWith(0, undefined);
    expect(host.isAlive()).toBe(false);
  });

  it('write/resize/kill are no-ops after exit', () => {
    const fake = makeFakeHandle();
    const host = new PtyHost(
      {
        agentId: 1,
        command: 'x',
        args: [],
        cwd: '/',
        logger: logger(),
        spawn: () => fake.handle,
      },
      { onData: () => {}, onExit: () => {} },
    );
    fake.fire.exit(0);
    host.write('after');
    host.resize(80, 24);
    host.kill();
    expect(fake.recorded.writes).toEqual([]);
    expect(fake.recorded.resizes).toEqual([]);
    expect(fake.recorded.signals).toEqual([]);
  });

  it('forwards write and resize while alive', () => {
    const fake = makeFakeHandle();
    const host = new PtyHost(
      {
        agentId: 1,
        command: 'x',
        args: [],
        cwd: '/',
        logger: logger(),
        spawn: () => fake.handle,
      },
      { onData: () => {}, onExit: () => {} },
    );
    host.write('hi');
    host.write(Buffer.from('bin'));
    host.resize(100, 30);
    host.kill('SIGINT');
    expect(fake.recorded.writes).toEqual(['hi', 'bin']);
    expect(fake.recorded.resizes).toEqual([[100, 30]]);
    expect(fake.recorded.signals).toEqual(['SIGINT']);
  });

  // WHY: pause/resume are a dormant flow-control capability (no caller gates on
  // backpressure today — the BroadcastSink ring is the OOM ceiling). They exist
  // so future per-agent flow control needs no re-plumbing. Test the contract:
  // forwarded while alive, no-op after exit, matching write/resize/kill.
  it('forwards pause/resume while alive', () => {
    const fake = makeFakeHandle();
    const host = new PtyHost(
      { agentId: 1, command: 'x', args: [], cwd: '/', logger: logger(), spawn: () => fake.handle },
      { onData: () => {}, onExit: () => {} },
    );
    host.pause();
    host.resume();
    expect(fake.recorded.flow).toEqual(['pause', 'resume']);
  });

  it('pause/resume are no-ops after exit', () => {
    const fake = makeFakeHandle();
    const host = new PtyHost(
      { agentId: 1, command: 'x', args: [], cwd: '/', logger: logger(), spawn: () => fake.handle },
      { onData: () => {}, onExit: () => {} },
    );
    fake.fire.exit(0);
    host.pause();
    host.resume();
    expect(fake.recorded.flow).toEqual([]);
  });
});
