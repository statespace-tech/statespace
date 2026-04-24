import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, "..");
const CLI = join(ROOT, "dist/cli.js");
const { DEFAULT_BASE_URL } = await import(join(ROOT, "dist/constants.js"));
const URL = process.env.TEST_URL ?? DEFAULT_BASE_URL;

function assertResult(r) {
  assert.equal(typeof r.url, "string");
  assert.equal(typeof r.site, "string");
  assert.equal(typeof r.title, "string");
  assert.equal(typeof r.snippet, "string");
}

// --- CLI ---

test("CLI: returns JSON array of results", async () => {
  const out = await new Promise((resolve, reject) => {
    const proc = spawn("node", [CLI, "search", "openai embeddings", "--limit", "3", "--url", URL]);
    let buf = "";
    proc.stdout.on("data", d => buf += d);
    proc.stderr.on("data", d => process.stderr.write(d));
    proc.on("close", code => code === 0 ? resolve(buf) : reject(new Error(`exit ${code}`)));
  });
  const results = JSON.parse(out);
  assert.ok(Array.isArray(results));
  assert.ok(results.length > 0);
  results.forEach(assertResult);
});

test("CLI: site: query syntax works", async () => {
  const out = await new Promise((resolve, reject) => {
    const proc = spawn("node", [CLI, "search", "openai: embeddings", "--limit", "3", "--url", URL]);
    let buf = "";
    proc.stdout.on("data", d => buf += d);
    proc.stderr.on("data", d => process.stderr.write(d));
    proc.on("close", code => code === 0 ? resolve(buf) : reject(new Error(`exit ${code}`)));
  });
  const results = JSON.parse(out);
  assert.ok(Array.isArray(results));
  results.forEach(assertResult);
});

// --- SDK ---

test("SDK: returns array of results", async () => {
  const { search } = await import(join(ROOT, "dist/sdk.js"));
  const results = await search("openai embeddings", { limit: 3, baseUrl: URL });
  assert.ok(Array.isArray(results));
  assert.ok(results.length > 0);
  results.forEach(assertResult);
});

test("SDK: throws on empty query", async () => {
  const { search } = await import(join(ROOT, "dist/sdk.js"));
  await assert.rejects(() => search("", { baseUrl: URL }));
});

// --- MCP ---

function mcpSession(args) {
  return new Promise((resolve, reject) => {
    const proc = spawn("node", [CLI, "mcp", "--url", URL, ...args]);
    const responses = [];
    let buf = "";

    proc.stdout.on("data", d => {
      buf += d.toString();
      const lines = buf.split("\n");
      buf = lines.pop();
      for (const line of lines) {
        if (line.trim()) responses.push(JSON.parse(line));
      }
    });
    proc.stderr.on("data", d => process.stderr.write(d));

    const messages = [
      { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "test", version: "0.0.1" } } },
      { jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "search", arguments: { q: "openai embeddings", limit: 2 } } },
    ];

    let i = 0;
    const send = () => {
      if (i >= messages.length) { setTimeout(() => { proc.kill(); resolve(responses); }, 400); return; }
      proc.stdin.write(JSON.stringify(messages[i++]) + "\n");
      setTimeout(send, 300);
    };
    send();
    proc.on("error", reject);
  });
}

test("MCP: initialize and search tool returns results", async () => {
  const responses = await mcpSession([]);
  const init = responses.find(r => r.id === 1);
  assert.ok(init?.result?.serverInfo?.name === "statespace");

  const call = responses.find(r => r.id === 2);
  assert.ok(call?.result?.content?.[0]?.type === "text");
  const results = JSON.parse(call.result.content[0].text);
  assert.ok(Array.isArray(results));
  assert.ok(results.length > 0);
  results.forEach(assertResult);
});
