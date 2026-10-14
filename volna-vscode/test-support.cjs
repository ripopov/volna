const traces = [
  {
    name: "picorv32.vtr",
    source: "volna/examples/picorv32.vtr",
    theme: "Default Dark Modern",
    viewport: [0, 10000000],
  },
  {
    name: "values.fst",
    source: "volna-trace/tests/fixtures/values.fst",
    theme: "Default Light Modern",
    viewport: [5, 20],
  },
];

async function until(check, description, timeout = 30000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await check();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`Timed out waiting for ${description}`);
}

module.exports = { traces, until };
