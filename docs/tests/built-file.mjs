import { stat } from 'node:fs/promises';
import { resolve } from 'node:path';

export const base = `/${(process.env.DOCS_BASE || '/').split('/').filter(Boolean).join('/')}`.replace(/\/$/, '') + '/';

export async function builtFile(root, path) {
  if (!path.startsWith(base)) throw new Error('outside base');
  let file = resolve(root, path.slice(base.length));
  if (!file.startsWith(root) || (file !== root && !file.startsWith(root.replace(/\/$/, '') + '/')))
    throw new Error('outside output');
  if ((await stat(file)).isDirectory()) file += '/index.html';
  return file;
}
