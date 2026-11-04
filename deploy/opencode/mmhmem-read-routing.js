// mmhmem read-routing bridge: installed only after explicit user opt-in.
import { execFile } from 'node:child_process';
const binary = __MMHMEM_BINARY__;
const maxInput = 1024 * 1024;
function run(binary, input, cwd) {
  return new Promise((resolve) => {
    try {
      const child = execFile(binary, ['hook', 'opencode'], {
        cwd, timeout: 2000, maxBuffer: 16 * 1024, windowsHide: true,
        encoding: 'utf8',
      }, (error, stdout) => resolve(error ? undefined : stdout));
      child.stdin.on('error', () => {});
      child.stdin.end(input);
    } catch { resolve(undefined); }
  });
}
export function createReadHook(binary, cwd, invoke = run) {
  return async (event) => {
    let reason;
    try {
      if (!event || !['read', 'shell'].includes(event.tool)) return;
      const input = JSON.stringify({ tool: event.tool, sessionID: event.sessionID, input: event.input, cwd });
      if (Buffer.byteLength(input, 'utf8') > maxInput) return;
      const output = await invoke(binary, input, cwd);
      if (!output) return;
      const answer = JSON.parse(output);
      if (answer?.decision === 'deny' && typeof answer.reason === 'string' && answer.reason.length) reason = answer.reason;
    } catch { return; }
    if (reason) throw new Error(reason);
  };
}
export default {
  id: 'mmhmem-read-routing',
  async setup(ctx) {
    try { await ctx.tool.hook('execute.before', createReadHook(binary, ctx.location.directory)); }
    catch { /* A failed registration also leaves reads alone. */ }
  },
};
