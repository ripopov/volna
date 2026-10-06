import { cp, mkdir } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const docs = fileURLToPath(new URL('../', import.meta.url));
const root = fileURLToPath(new URL('../../', import.meta.url));
const pkg = fileURLToPath(new URL('../public/viewer/pkg/', import.meta.url));
const examples = fileURLToPath(new URL('../public/examples/', import.meta.url));

function run(command, args, cwd, env = process.env) {
  const result = spawnSync(command, args, { cwd, stdio: 'inherit', env });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited with ${result.status}`);
}

await mkdir(pkg, { recursive: true });
await mkdir(examples, { recursive: true });
// wasm_thread currently uses stdarch_wasm_atomic_wait behind an unstable gate.
run('cargo', ['build', '-p', 'volna', '--lib', '--target', 'wasm32-unknown-unknown', '--release', '--locked'], root,
  { ...process.env, RUSTC_BOOTSTRAP: '1' });
run('wasm-bindgen', [
  '--target', 'web', '--out-dir', pkg,
  fileURLToPath(new URL('../../target/wasm32-unknown-unknown/release/volna.wasm', import.meta.url)),
], docs);
for (const name of ['landing.vtr', 'landing.vtr.volna.json']) {
  await cp(new URL(`../../volna/examples/${name}`, import.meta.url), new URL(`../public/examples/${name}`, import.meta.url));
}
