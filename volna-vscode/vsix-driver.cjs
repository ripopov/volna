const assert = require("node:assert/strict");
const fs = require("node:fs/promises");
const path = require("node:path");
const vscode = require("vscode");
const { traces, until } = require("./test-support.cjs");

const exists = (file) =>
  fs.access(file).then(
    () => true,
    () => false,
  );

exports.run = async function () {
  const root = process.env.VOLNA_VSIX_TEST_DIR;
  try {
    const extension = vscode.extensions.getExtension(
      process.env.VOLNA_EXTENSION_ID,
    );
    assert.ok(extension, "the VSIX is installed");
    await extension.activate();
    await vscode.commands.executeCommand("workbench.action.closeSidebar");
    for (const [index, { name, theme }] of traces.entries()) {
      await vscode.workspace
        .getConfiguration("workbench")
        .update("colorTheme", theme, vscode.ConfigurationTarget.Global);
      const uri = vscode.Uri.file(path.join(root, name));
      await vscode.commands.executeCommand("vscode.open", uri, {
        preview: false,
      });
      const input = await until(
        () => {
          const input = vscode.window.tabGroups.activeTabGroup.activeTab?.input;
          return (
            input instanceof vscode.TabInputCustom &&
            input.uri.toString() === uri.toString() &&
            input
          );
        },
        `${name} default custom editor`,
        10000,
      );
      assert.equal(input.viewType, "volna.waveform");
      await fs.writeFile(
        path.join(root, "current.json"),
        JSON.stringify({ index, name }),
      );
      await until(
        () => exists(path.join(root, `trace-loaded-${index}`)),
        `${name} loaded trace`,
        90000,
      );
      await vscode.commands.executeCommand("volna.newPanel");
      await until(
        () => exists(path.join(root, `passed-${index}`)),
        `${name} rendered viewer`,
        60000,
      );
      await vscode.commands.executeCommand("workbench.action.closeAllEditors");
    }
    await fs.writeFile(path.join(root, "done"), "ok");
  } catch (error) {
    await fs.writeFile(path.join(root, "driver-error.txt"), error.stack);
    throw error;
  }
};
