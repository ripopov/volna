# VS Code theme fixtures

The four text snapshots record resolved webview CSS variables from VS Code
1.137 for Dark Modern, Light Modern, Default High Contrast, and Default High
Contrast Light. Their headers record provenance and theme kind. Core tests
feed them through `theme::vscode::host_palette` to check the mapping into
viewer colours. `../custom-palette.json` covers the host-neutral JSON parser.
