import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const source = (await readFile(new URL('../deploy/opencode/mmhmem-read-routing.js', import.meta.url), 'utf8')).replace('__MMHMEM_BINARY__', JSON.stringify('/binary'));
const { createReadHook } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const event = { tool:'read', sessionID:'s', input:{path:'big.rs'} };
test('only valid deny blocks; transport failures pass', async () => {
 for (const result of [undefined, '', '{', '{}', '{"decision":"pass"}', '{"decision":"deny"}']) {
  await createReadHook('/binary', '/project', async () => result)(event);
 }
 await createReadHook('/binary', '/project', async () => {throw new Error('missing')})(event);
 await assert.rejects(createReadHook('/binary', '/project', async () => '{"decision":"deny","reason":"redirect"}')(event), /redirect/);
});
test('forwards only read/shell with cwd and bounded input', async () => {
 const calls=[];
 const hook=createReadHook('/binary', '/project', async (...args) => {calls.push(args); return ''});
 await hook(event); await hook({...event,tool:'shell'}); await hook({...event,tool:'edit'});
 await hook({...event,input:{path:'x'.repeat(1024*1024)}});
 assert.equal(calls.length,2);
 assert.equal(calls[0][0],'/binary');
 assert.deepEqual(JSON.parse(calls[0][1]),{...event,cwd:'/project'});
});
