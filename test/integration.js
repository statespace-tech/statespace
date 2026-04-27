import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, "..");
const CLI = join(ROOT, "dist/cli.js");
const { DEFAULT_BASE_URL } = await import(join(ROOT, "dist/constants.js"));
const BASE_URL = process.env.TEST_URL ?? DEFAULT_BASE_URL;

function assertResult(r) {
  assert.equal(typeof r.url, "string");
  assert.equal(typeof r.site, "string");
  assert.equal(typeof r.title, "string");
  assert.equal(typeof r.snippet, "string");
}

function cli(...args) {
  return new Promise((resolve, reject) => {
    const proc = spawn("node", [CLI, "search", ...args, "--url", BASE_URL]);
    let stdout = "";
    proc.stdout.on("data", d => (stdout += d));
    proc.stderr.on("data", d => process.stderr.write(d));
    proc.on("close", code =>
      code === 0 ? resolve(stdout) : reject(new Error(`exit ${code}`))
    );
  });
}

// --- CLI ---

test('CLI: "mcp server setup"', async () => {
  const results = JSON.parse(await cli("mcp server setup"));
  assert.ok(Array.isArray(results));
  assert.ok(results.length > 0);
  assert.ok(results.length <= 10);
  results.forEach(assertResult);
});

test('CLI: "stripe: webhook verification" --limit 5', async () => {
  const results = JSON.parse(await cli("stripe: webhook verification", "--limit", "5"));
  assert.ok(Array.isArray(results));
  assert.ok(results.length > 0);
  assert.ok(results.length <= 5);
  results.forEach(assertResult);
});

test('CLI: "redis connection pooling" --limit 10 --offset 3', async () => {
  const results = JSON.parse(await cli("redis connection pooling", "--limit", "10", "--offset", "3"));
  assert.ok(Array.isArray(results));
  assert.ok(results.length <= 10);
  results.forEach(assertResult);
});

test('CLI: "anthropic: tool use function calling" --limit 5 --human', async () => {
  const out = await cli("anthropic: tool use function calling", "--limit", "5", "--human");
  const lines = out.trim().split("\n").filter(Boolean);
  assert.ok(lines.length > 0);
  assert.ok(lines.length <= 5);
});

// --- SDK ---

const { search } = await import(join(ROOT, "dist/sdk.js"));

test('SDK: search("mcp server setup")', async () => {
  const results = await search("mcp server setup", { baseUrl: BASE_URL });
  assert.ok(Array.isArray(results));
  assert.ok(results.length > 0);
  assert.ok(results.length <= 10);
  results.forEach(assertResult);
});

test('SDK: search("stripe: webhook verification", { limit: 5 })', async () => {
  const results = await search("stripe: webhook verification", { limit: 5, baseUrl: BASE_URL });
  assert.ok(Array.isArray(results));
  assert.ok(results.length > 0);
  assert.ok(results.length <= 5);
  results.forEach(assertResult);
});

test('SDK: search("redis connection pooling", { limit: 10, offset: 3 })', async () => {
  const results = await search("redis connection pooling", { limit: 10, offset: 3, baseUrl: BASE_URL });
  assert.ok(Array.isArray(results));
  assert.ok(results.length <= 10);
  results.forEach(assertResult);
});

// --- MCP ---

function mcpSearch(q, limit, offset) {
  return new Promise((resolve, reject) => {
    const proc = spawn("node", [CLI, "mcp", "--url", BASE_URL]);
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

    const args = { q };
    if (limit !== undefined) args.limit = limit;
    if (offset !== undefined) args.offset = offset;

    const messages = [
      { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "test", version: "0.0.1" } } },
      { jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "search", arguments: args } },
    ];

    let i = 0;
    const send = () => {
      if (i >= messages.length) {
        setTimeout(() => { proc.kill(); resolve(responses); }, 400);
        return;
      }
      proc.stdin.write(JSON.stringify(messages[i++]) + "\n");
      setTimeout(send, 300);
    };
    send();
    proc.on("error", reject);
  });
}

function mcpResults(responses) {
  const call = responses.find(r => r.id === 2);
  assert.equal(call?.result?.content?.[0]?.type, "text");
  return JSON.parse(call.result.content[0].text);
}

test('MCP: search "mcp server setup"', async () => {
  const results = mcpResults(await mcpSearch("mcp server setup"));
  assert.ok(Array.isArray(results));
  assert.ok(results.length > 0);
  assert.ok(results.length <= 10);
  results.forEach(assertResult);
});

test('MCP: search "stripe: webhook verification" limit 5', async () => {
  const results = mcpResults(await mcpSearch("stripe: webhook verification", 5));
  assert.ok(Array.isArray(results));
  assert.ok(results.length > 0);
  assert.ok(results.length <= 5);
  results.forEach(assertResult);
});

test('MCP: search "redis connection pooling" limit 10 offset 3', async () => {
  const results = mcpResults(await mcpSearch("redis connection pooling", 10, 3));
  assert.ok(Array.isArray(results));
  assert.ok(results.length <= 10);
  results.forEach(assertResult);
});
