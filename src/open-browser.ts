import { spawn } from 'node:child_process';

function launcher(): { command: string; args: string[] } {
  if (process.platform === 'darwin') return { command: 'open', args: [] };
  if (process.platform === 'win32') return { command: 'cmd', args: ['/c', 'start', ''] };
  return { command: 'xdg-open', args: [] };
}

/**
 * Best effort: a headless box or a missing xdg-open is not a reason to fail the
 * review, since the URL is printed either way.
 */
export function openBrowser(url: string): void {
  const { command, args } = launcher();
  try {
    const child = spawn(command, [...args, url], { detached: true, stdio: 'ignore' });
    child.on('error', () => undefined);
    child.unref();
  } catch {
    // Ignored: the caller has already printed the URL.
  }
}
