import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { installSupervisor } from '../src/supervisor.js';

const NODE = '/usr/bin/node';
const SCRIPT = '/home/user/.local/share/pixel-agents/server.js';

let tmpHome: string;
let origHome: string | undefined;

beforeEach(() => {
  tmpHome = fs.mkdtempSync(path.join(os.tmpdir(), 'pa-sup-'));
  origHome = process.env.HOME;
  process.env.HOME = tmpHome;
});

afterEach(() => {
  if (origHome !== undefined) process.env.HOME = origHome;
  try {
    fs.rmSync(tmpHome, { recursive: true, force: true });
  } catch {
    // best effort
  }
});

describe('installSupervisor — linux (systemd)', () => {
  it('writes service file with correct ExecStart', () => {
    const result = installSupervisor({ nodePath: NODE, scriptPath: SCRIPT, platform: 'linux' });
    expect(result.configPath).toMatch(/systemd\/user\/pixel-agents\.service$/);
    const content = fs.readFileSync(result.configPath, 'utf-8');
    expect(content).toContain(`ExecStart=${NODE} ${SCRIPT} --foreground`);
    expect(content).toContain('Restart=on-failure');
    expect(content).toContain('SuccessExitStatus=0');
  });

  it('returns correct activate command', () => {
    const result = installSupervisor({ nodePath: NODE, scriptPath: SCRIPT, platform: 'linux' });
    expect(result.activateCommand).toBe('systemctl --user enable --now pixel-agents.service');
  });

  it('creates parent directories if absent', () => {
    const result = installSupervisor({ nodePath: NODE, scriptPath: SCRIPT, platform: 'linux' });
    expect(fs.existsSync(path.dirname(result.configPath))).toBe(true);
  });

  it('alreadyExisted = false on first install, true on second', () => {
    const r1 = installSupervisor({ nodePath: NODE, scriptPath: SCRIPT, platform: 'linux' });
    expect(r1.alreadyExisted).toBe(false);
    const r2 = installSupervisor({ nodePath: NODE, scriptPath: SCRIPT, platform: 'linux' });
    expect(r2.alreadyExisted).toBe(true);
  });
});

describe('installSupervisor — darwin (launchd)', () => {
  it('writes plist with node + script in ProgramArguments', () => {
    const result = installSupervisor({ nodePath: NODE, scriptPath: SCRIPT, platform: 'darwin' });
    expect(result.configPath).toMatch(/LaunchAgents\/com\.pixelagents\.daemon\.plist$/);
    const content = fs.readFileSync(result.configPath, 'utf-8');
    expect(content).toContain(`<string>${NODE}</string>`);
    expect(content).toContain(`<string>${SCRIPT}</string>`);
    expect(content).toContain('<key>SuccessfulExit</key><false/>');
    expect(content).toContain('<key>Crashed</key><true/>');
  });

  it('activate command references the config path', () => {
    const result = installSupervisor({ nodePath: NODE, scriptPath: SCRIPT, platform: 'darwin' });
    expect(result.activateCommand).toBe(`launchctl load -w ${result.configPath}`);
  });
});

describe('installSupervisor — win32 (Scheduled Task)', () => {
  it('writes XML with Command + Arguments', () => {
    const result = installSupervisor({ nodePath: NODE, scriptPath: SCRIPT, platform: 'win32' });
    expect(result.configPath).toMatch(/pixel-agents-task\.xml$/);
    const content = fs.readFileSync(result.configPath, 'utf-8');
    expect(content).toContain(`<Command>${NODE}</Command>`);
    expect(content).toContain(`"${SCRIPT}" --foreground`);
    expect(content).toContain('<RestartOnFailure>');
    expect(content).toContain('<LogonTrigger>');
  });

  it('activate command uses schtasks with the xml path', () => {
    const result = installSupervisor({ nodePath: NODE, scriptPath: SCRIPT, platform: 'win32' });
    expect(result.activateCommand).toContain('schtasks /Create /TN "PixelAgents"');
    expect(result.activateCommand).toContain(result.configPath);
  });
});

describe('installSupervisor — errors', () => {
  it('throws on unsupported platform', () => {
    expect(() =>
      installSupervisor({ nodePath: NODE, scriptPath: SCRIPT, platform: 'freebsd' }),
    ).toThrow('Unsupported platform: freebsd');
  });
});
