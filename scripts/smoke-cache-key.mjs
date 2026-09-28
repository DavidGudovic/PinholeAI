#!/usr/bin/env node
// Print cache-key parts for the engine smoke test (CI appends them to $GITHUB_OUTPUT):
//   model=<hash of the smoke model URL + sha256 from models.yaml `test_models:`>
//   engine=<hash of the stable_diffusion_cpp section of engine.yaml>
// Dependency-free: a tiny indentation-based scan instead of a YAML parser.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const DEFAULT_MODEL_URL = "https://huggingface.co/Comfy-Org/stable-diffusion-v1-5-archive/resolve/main/v1-5-pruned-emaonly-fp16.safetensors";

const read = (p) => {
  try { return fs.readFileSync(path.join(ROOT, p), "utf8"); } catch { return ""; }
};

/** Text of the top-level YAML block `key:` (until the next top-level key). */
function block(text, key) {
  const lines = text.split(/\r?\n/);
  const start = lines.findIndex((l) => new RegExp(`^${key}:\\s*(#.*)?$`).test(l));
  if (start < 0) return "";
  const out = [];
  for (let i = start + 1; i < lines.length; i++) {
    if (/^\S/.test(lines[i]) && !/^#/.test(lines[i])) break;
    out.push(lines[i]);
  }
  return out.join("\n");
}

const h = (s) => crypto.createHash("sha256").update(s).digest("hex").slice(0, 16);

const tm = block(read("config/models.yaml"), "test_models");
const url = (tm.match(/url:\s*["']?([^"'\s,}]+)/) || [])[1] || DEFAULT_MODEL_URL;
const sha = (tm.match(/sha256:\s*["']?([0-9a-fA-F]{64}|TODO)/) || [])[1] || "";
const engine = block(read("config/engine.yaml"), "stable_diffusion_cpp");

console.log(`model=${h(url + sha)}`);
console.log(`engine=${h(engine)}`);
console.log(`model_url=${url}`);
