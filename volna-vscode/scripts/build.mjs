import { cp } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { buildWebViewer } from "../../volna/tools/build-web.mjs";

const media = fileURLToPath(new URL("../media/", import.meta.url));
await buildWebViewer(media);
for (const name of ["webview.mjs", "webview.css", "theme.mjs"]) {
  await cp(
    new URL(`../${name}`, import.meta.url),
    new URL(`../media/${name}`, import.meta.url),
  );
}
await cp(
  new URL("../../docs/design-system/", import.meta.url),
  new URL("../media/design/", import.meta.url),
  {
    recursive: true,
    filter: (source) => !source.endsWith(".md"),
  },
);
