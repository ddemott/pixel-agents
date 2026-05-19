import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';

export interface InstallResult {
  configPath: string;
  activateCommand: string;
  alreadyExisted: boolean;
}

interface SupervisorOpts {
  /** Override the node executable path (test seam). Default: process.execPath */
  nodePath?: string;
  /** Override the daemon script path (test seam). Default: resolved process.argv[1] */
  scriptPath?: string;
  /** Override platform detection (test seam). Default: process.platform */
  platform?: string;
}

/**
 * Write the OS-appropriate supervisor config for the Pixel Agents daemon.
 * Never auto-enables — prints the activation command for the user to run.
 * Restart policy: on-failure only (clean exit = user stopped it; supervisor stays down).
 */
export function installSupervisor(opts: SupervisorOpts = {}): InstallResult {
  const nodePath = opts.nodePath ?? process.execPath;
  const scriptPath = opts.scriptPath ?? path.resolve(process.argv[1]);
  const platform = opts.platform ?? process.platform;

  switch (platform) {
    case 'linux':
      return installSystemd(nodePath, scriptPath);
    case 'darwin':
      return installLaunchd(nodePath, scriptPath);
    case 'win32':
      return installWindowsTask(nodePath, scriptPath);
    default:
      throw new Error(
        `Unsupported platform: ${platform}. Supported: linux, darwin, win32.\n` +
          `To configure manually see https://github.com/pixel-agents/pixel-agents/tree/main/docs`,
      );
  }
}

function installSystemd(nodePath: string, scriptPath: string): InstallResult {
  const configDir = path.join(os.homedir(), '.config', 'systemd', 'user');
  const configPath = path.join(configDir, 'pixel-agents.service');

  const content = [
    '[Unit]',
    'Description=Pixel Agents daemon (per-user)',
    'After=default.target',
    '',
    '[Service]',
    'Type=simple',
    `ExecStart=${nodePath} ${scriptPath} --foreground`,
    'Restart=on-failure',
    'RestartSec=2s',
    'SuccessExitStatus=0',
    'KillMode=mixed',
    'KillSignal=SIGTERM',
    'TimeoutStopSec=10',
    'Environment=NODE_ENV=production',
    '',
    '[Install]',
    'WantedBy=default.target',
    '',
  ].join('\n');

  const alreadyExisted = fs.existsSync(configPath);
  fs.mkdirSync(configDir, { recursive: true });
  fs.writeFileSync(configPath, content, 'utf-8');

  return {
    configPath,
    activateCommand: 'systemctl --user enable --now pixel-agents.service',
    alreadyExisted,
  };
}

function installLaunchd(nodePath: string, scriptPath: string): InstallResult {
  const configDir = path.join(os.homedir(), 'Library', 'LaunchAgents');
  const configPath = path.join(configDir, 'com.pixelagents.daemon.plist');

  const content = [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">',
    '<plist version="1.0">',
    '<dict>',
    '  <key>Label</key><string>com.pixelagents.daemon</string>',
    '  <key>ProgramArguments</key><array>',
    `    <string>${nodePath}</string>`,
    `    <string>${scriptPath}</string>`,
    '    <string>--foreground</string>',
    '  </array>',
    '  <key>KeepAlive</key>',
    '  <dict><key>SuccessfulExit</key><false/><key>Crashed</key><true/></dict>',
    '  <key>ThrottleInterval</key><integer>2</integer>',
    '  <key>StandardOutPath</key><string>/tmp/pixel-agents.out</string>',
    '  <key>StandardErrorPath</key><string>/tmp/pixel-agents.err</string>',
    '</dict>',
    '</plist>',
    '',
  ].join('\n');

  const alreadyExisted = fs.existsSync(configPath);
  fs.mkdirSync(configDir, { recursive: true });
  fs.writeFileSync(configPath, content, 'utf-8');

  return {
    configPath,
    activateCommand: `launchctl load -w ${configPath}`,
    alreadyExisted,
  };
}

function installWindowsTask(nodePath: string, scriptPath: string): InstallResult {
  // Write XML to home dir; user runs schtasks to register (avoids UAC complications).
  const configPath = path.join(os.homedir(), 'pixel-agents-task.xml');

  // schtasks requires UTF-16 LE. We write UTF-8 with a BOM note in the comment;
  // users who need strict UTF-16 can re-encode before running schtasks.
  const content = [
    '<?xml version="1.0" encoding="UTF-16"?>',
    '<Task xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">',
    '  <Triggers>',
    '    <LogonTrigger><Enabled>true</Enabled></LogonTrigger>',
    '  </Triggers>',
    '  <Actions>',
    '    <Exec>',
    `      <Command>${nodePath}</Command>`,
    `      <Arguments>"${scriptPath}" --foreground</Arguments>`,
    '    </Exec>',
    '  </Actions>',
    '  <Settings>',
    '    <RestartOnFailure>',
    '      <Interval>PT5S</Interval>',
    '      <Count>3</Count>',
    '    </RestartOnFailure>',
    '    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>',
    '    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>',
    '  </Settings>',
    '</Task>',
    '',
  ].join('\n');

  const alreadyExisted = fs.existsSync(configPath);
  fs.writeFileSync(configPath, content, 'utf-8');

  return {
    configPath,
    activateCommand: `schtasks /Create /TN "PixelAgents" /XML "${configPath}"`,
    alreadyExisted,
  };
}
