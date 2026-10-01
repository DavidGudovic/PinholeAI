#!/usr/bin/env node
// Check a release's SHA256SUMS.txt.sig (`tauri signer sign`) against the public key built
// into the app (src-tauri/update-key.pub), the way the app does before an in-app update
// (pinhole-core update::verify_sums). The Release workflow runs it so a release signed with
// the wrong key never ships.
//
//   node scripts/verify-update-signature.mjs <SHA256SUMS.txt> <SHA256SUMS.txt.sig> <update-key.pub>
import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";

const [file, sigFile, keyFile] = process.argv.slice(2);
if (!keyFile) {
  console.error("usage: verify-update-signature.mjs <file> <file.sig> <update-key.pub>");
  process.exit(2);
}

// Both files are base64 of a minisign text file; return its lines.
const lines = (path) =>
  Buffer.from(readFileSync(path, "utf8").replace(/\s+/g, ""), "base64").toString("utf8").split("\n");

function fail(message) {
  console.error(`::error::${message}`);
  process.exit(1);
}

const key = Buffer.from(lines(keyFile)[1] ?? "", "base64");
if (key.length !== 42 || key.subarray(0, 2).toString() !== "Ed") fail("update-key.pub is not a minisign public key");
const [, sigLine = "", trusted = "", globalLine = ""] = lines(sigFile);
const sig = Buffer.from(sigLine, "base64");
const global = Buffer.from(globalLine, "base64");
if (sig.length !== 74 || global.length !== 64 || !trusted.startsWith("trusted comment: ")) {
  fail("the signature file can't be read");
}
if (!sig.subarray(2, 10).equals(key.subarray(2, 10))) {
  fail("the file was signed with a different key than src-tauri/update-key.pub");
}

const publicKey = createPublicKey({
  key: { kty: "OKP", crv: "Ed25519", x: key.subarray(10).toString("base64url") },
  format: "jwk",
});
const alg = sig.subarray(0, 2).toString();
const data = readFileSync(file);
// The app accepts only prehashed signatures (minisign "ED"), so the release gate does too.
if (alg !== "ED") fail(`signature algorithm ${alg} isn't accepted by the app (expected ED)`);
const message = createHash("blake2b512").update(data).digest();
if (!verify(null, message, publicKey, sig.subarray(10))) fail(`${file}: signature does not match`);
const comment = Buffer.from(trusted.slice("trusted comment: ".length));
if (!verify(null, Buffer.concat([sig.subarray(10), comment]), publicKey, global)) {
  fail(`${file}: trusted comment signature does not match`);
}
console.log(`${file}: signature OK (key ${Buffer.from(key.subarray(2, 10)).reverse().toString("hex").toUpperCase()})`);
