import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * vi.resetModules() + dynamic imports so module-level constants (PIXEL_AGENTS_DIR)
 * re-derive from the fresh HOME environment on every test, matching the
 * pattern used in resume.test.ts and agentsRegistry.test.ts.
 */

const SESSION_ID = 'bbbbbbbb-0000-0000-0000-000000000001';

let tmpHome: string;
let origHome: string | undefined;

beforeEach(() => {
  tmpHome = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-poller-'));
  origHome = process.env.HOME;
  process.env.HOME = tmpHome;
  vi.resetModules();
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
  if (origHome !== undefined) process.env.HOME = origHome;
  try {
    fs.rmSync(tmpHome, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

function writeJsonl(filePath: string, lines: string[]): void {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, lines.map((l) => l + '\n').join(''));
}

function appendJsonl(filePath: string, lines: string[]): void {
  fs.appendFileSync(filePath, lines.map((l) => l + '\n').join(''));
}

async function makePoller() {
  const [{ JsonlPoller }, { RecordingSink }, { createNullLogger }] = await Promise.all([
    import('../../src/agents/jsonlPoller.js'),
    import('../../../src/messageSender.js'),
    import('../../src/logging/logger.js'),
  ]);
  const sink = new RecordingSink();
  const logger = createNullLogger();
  const poller = new JsonlPoller(sink, logger);
  return { poller, sink };
}

describe('JsonlPoller', () => {
  it('emits agentToolStart when tool_use block arrives', async () => {
    const { poller, sink } = await makePoller();
    const jsonlPath = path.join(tmpHome, 'transcript.jsonl');
    writeJsonl(jsonlPath, [
      JSON.stringify({
        type: 'assistant',
        message: {
          content: [
            { type: 'tool_use', id: 'tool-1', name: 'Read', input: { file_path: '/foo.ts' } },
          ],
        },
      }),
    ]);

    poller.start(1, SESSION_ID, tmpHome, jsonlPath, 0);
    await vi.advanceTimersByTimeAsync(600);

    const starts = sink.events.filter((e) => e.type === 'agentToolStart');
    expect(starts.length).toBe(1);
    expect(starts[0]).toMatchObject({ id: 1, toolId: 'tool-1', toolName: 'Read' });

    poller.stop(1);
  });

  it('emits agentToolDone when tool_result arrives', async () => {
    const { poller, sink } = await makePoller();
    const jsonlPath = path.join(tmpHome, 'transcript.jsonl');
    writeJsonl(jsonlPath, [
      JSON.stringify({
        type: 'assistant',
        message: {
          content: [{ type: 'tool_use', id: 'tool-2', name: 'Bash', input: { command: 'ls' } }],
        },
      }),
      JSON.stringify({
        type: 'user',
        message: { content: [{ type: 'tool_result', tool_use_id: 'tool-2' }] },
      }),
    ]);

    poller.start(2, SESSION_ID, tmpHome, jsonlPath, 0);
    await vi.advanceTimersByTimeAsync(600);

    // agentToolDone is delayed by TOOL_DONE_DELAY_MS (300 ms) in transcriptParser
    await vi.advanceTimersByTimeAsync(400);

    const dones = sink.events.filter((e) => e.type === 'agentToolDone');
    expect(dones.length).toBe(1);
    expect(dones[0]).toMatchObject({ id: 2, toolId: 'tool-2' });

    poller.stop(2);
  });

  it('emits agentStatus waiting on turn_duration', async () => {
    const { poller, sink } = await makePoller();
    const jsonlPath = path.join(tmpHome, 'transcript.jsonl');
    writeJsonl(jsonlPath, [
      JSON.stringify({ type: 'system', subtype: 'turn_duration', duration_ms: 1000 }),
    ]);

    poller.start(3, SESSION_ID, tmpHome, jsonlPath, 0);
    await vi.advanceTimersByTimeAsync(600);

    const statuses = sink.events.filter((e) => e.type === 'agentStatus');
    expect(statuses.some((e) => e['status'] === 'waiting')).toBe(true);

    poller.stop(3);
  });

  it('seeds fileOffset so old content is skipped on revival', async () => {
    const { poller, sink } = await makePoller();
    const jsonlPath = path.join(tmpHome, 'transcript.jsonl');
    const oldLine = JSON.stringify({
      type: 'assistant',
      message: { content: [{ type: 'tool_use', id: 'old-tool', name: 'Read', input: {} }] },
    });
    writeJsonl(jsonlPath, [oldLine]);
    const seedOffset = fs.statSync(jsonlPath).size;

    // New content appended after seeding
    const newLine = JSON.stringify({ type: 'system', subtype: 'turn_duration', duration_ms: 500 });
    appendJsonl(jsonlPath, [newLine]);

    poller.start(4, SESSION_ID, tmpHome, jsonlPath, seedOffset);
    await vi.advanceTimersByTimeAsync(600);

    // Old tool_use should NOT have been re-emitted
    const starts = sink.events.filter((e) => e.type === 'agentToolStart');
    expect(starts.length).toBe(0);

    // turn_duration after seed IS emitted
    const statuses = sink.events.filter((e) => e.type === 'agentStatus');
    expect(statuses.some((e) => e['status'] === 'waiting')).toBe(true);

    poller.stop(4);
  });

  it('picks up new content appended after start', async () => {
    const { poller, sink } = await makePoller();
    const jsonlPath = path.join(tmpHome, 'transcript.jsonl');
    writeJsonl(jsonlPath, []); // empty file

    poller.start(5, SESSION_ID, tmpHome, jsonlPath, 0);
    await vi.advanceTimersByTimeAsync(600); // poll with no content

    // Append a tool_use line
    appendJsonl(jsonlPath, [
      JSON.stringify({
        type: 'assistant',
        message: { content: [{ type: 'tool_use', id: 'late-tool', name: 'Grep', input: {} }] },
      }),
    ]);
    await vi.advanceTimersByTimeAsync(600);

    const starts = sink.events.filter((e) => e.type === 'agentToolStart');
    expect(starts.length).toBe(1);
    expect(starts[0]).toMatchObject({ toolId: 'late-tool' });

    poller.stop(5);
  });

  it('tolerates ENOENT gracefully (file not yet created)', async () => {
    const { poller, sink } = await makePoller();
    const jsonlPath = path.join(tmpHome, 'nonexistent.jsonl');

    poller.start(6, SESSION_ID, tmpHome, jsonlPath, 0);
    await vi.advanceTimersByTimeAsync(600);

    // No errors thrown, no events emitted for a missing file
    expect(sink.events.length).toBe(0);

    poller.stop(6);
  });

  it('suppresses heuristic timers when hookDelivered is set', async () => {
    const { poller, sink } = await makePoller();
    const jsonlPath = path.join(tmpHome, 'transcript.jsonl');
    writeJsonl(jsonlPath, [
      JSON.stringify({
        type: 'assistant',
        message: {
          content: [{ type: 'tool_use', id: 't1', name: 'Bash', input: { command: 'x' } }],
        },
      }),
    ]);

    poller.start(7, SESSION_ID, tmpHome, jsonlPath, 0);
    poller.markHookDelivered(7);
    await vi.advanceTimersByTimeAsync(600);

    // agentToolStart should NOT be emitted when hookDelivered=true (suppressed for non-subagent tools)
    const starts = sink.events.filter((e) => e.type === 'agentToolStart');
    expect(starts.length).toBe(0);

    poller.stop(7);
  });

  it('stop clears the polling timer', async () => {
    const { poller, sink } = await makePoller();
    const jsonlPath = path.join(tmpHome, 'transcript.jsonl');
    writeJsonl(jsonlPath, []);

    poller.start(8, SESSION_ID, tmpHome, jsonlPath, 0);
    poller.stop(8);

    // Append content after stop — should NOT be picked up
    appendJsonl(jsonlPath, [
      JSON.stringify({ type: 'system', subtype: 'turn_duration', duration_ms: 100 }),
    ]);
    await vi.advanceTimersByTimeAsync(600);

    expect(sink.events.length).toBe(0);
  });

  it('stopAll clears all active pollers', async () => {
    const { poller, sink } = await makePoller();
    const p1 = path.join(tmpHome, 'a.jsonl');
    const p2 = path.join(tmpHome, 'b.jsonl');
    writeJsonl(p1, []);
    writeJsonl(p2, []);

    poller.start(10, 'sess-a', tmpHome, p1, 0);
    poller.start(11, 'sess-b', tmpHome, p2, 0);
    poller.stopAll();

    // Append after stopAll — neither picked up
    appendJsonl(p1, [JSON.stringify({ type: 'system', subtype: 'turn_duration', duration_ms: 1 })]);
    appendJsonl(p2, [JSON.stringify({ type: 'system', subtype: 'turn_duration', duration_ms: 1 })]);
    await vi.advanceTimersByTimeAsync(600);

    expect(sink.events.length).toBe(0);
  });
});
