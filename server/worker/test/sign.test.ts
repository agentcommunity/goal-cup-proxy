// The Worker's signatures must match, byte for byte, the ones the Rust code makes. Both sides read
// the same file, test/vectors/signing.json; crates/goalcup-protocol/tests/it/signing_vectors.rs
// asserts the same hex. The key is the Rust tests' key: Ed25519 seed = 32 bytes of 0x07.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { Signer, ackSigningBytes, receiptSigningBytes, type ReceiptClaims } from "../src/sign.ts";
import { fromHex, toHex } from "../src/util.ts";

const VECTORS = JSON.parse(readFileSync(join(dirname(fileURLToPath(import.meta.url)), "vectors", "signing.json"), "utf8")) as {
  seed_hex: string;
  public_key: string;
  receipt: { claims: ReceiptClaims; signature: string };
  ack: { entry_id: string; seq: number; hash: string; acked_at: string; server_signature: string };
};

const SEED = VECTORS.seed_hex;
const PUBLIC_KEY = VECTORS.public_key;
const RUST_RECEIPT_CLAIMS: ReceiptClaims = VECTORS.receipt.claims;
const RUST_RECEIPT_SIGNATURE = VECTORS.receipt.signature;
const RUST_ACK_SIGNATURE = VECTORS.ack.server_signature;

test("public key derived from the seed is the Rust one", async () => {
  const signer = await Signer.fromSeedHex(SEED);
  assert.equal(signer.publicKeyHex, PUBLIC_KEY);
});

test("start receipt reproduces the Rust signature byte for byte", async () => {
  const signer = await Signer.fromSeedHex(SEED);
  const receipt = await signer.signReceipt(RUST_RECEIPT_CLAIMS);
  assert.equal(receipt.public_key, PUBLIC_KEY);
  assert.equal(receipt.signature, RUST_RECEIPT_SIGNATURE);
  assert.deepEqual(receipt.claims, RUST_RECEIPT_CLAIMS);
});

test("acknowledgement reproduces the Rust signature byte for byte", async () => {
  const signer = await Signer.fromSeedHex(SEED);
  const a = VECTORS.ack;
  const ack = await signer.signAck(a.entry_id, a.seq, a.hash, a.acked_at);
  assert.deepEqual(ack, { ...a, server_signature: RUST_ACK_SIGNATURE });
});

test("signing bytes: domain tag, compact JSON, field order", () => {
  const text = new TextDecoder().decode(receiptSigningBytes(RUST_RECEIPT_CLAIMS));
  assert.ok(text.startsWith("goalcup-receipt-v1\0{\"entry_id\":\"e0123456789abcdef\",\"connector_id\":\"codex-cli\",\"workspace\":"));
  assert.ok(text.endsWith(`"connector_dir_sha256":"${"c".repeat(64)}"}`));
  const ack = new TextDecoder().decode(ackSigningBytes({ entry_id: "e1", seq: 3, hash: "h", acked_at: "t" }));
  assert.equal(ack, 'goalcup-ack-v1\0{"entry_id":"e1","seq":3,"hash":"h","acked_at":"t"}');
});

test("signatures verify with plain WebCrypto against the public key", async () => {
  const signer = await Signer.fromSeedHex(SEED);
  const receipt = await signer.signReceipt(RUST_RECEIPT_CLAIMS);
  const key = await crypto.subtle.importKey("raw", fromHex(PUBLIC_KEY), { name: "Ed25519" }, false, ["verify"]);
  const ok = await crypto.subtle.verify({ name: "Ed25519" }, key, fromHex(receipt.signature), receiptSigningBytes(RUST_RECEIPT_CLAIMS));
  assert.equal(ok, true);
  const tampered = { ...RUST_RECEIPT_CLAIMS, chain_head: "e".repeat(64) };
  assert.equal(await crypto.subtle.verify({ name: "Ed25519" }, key, fromHex(receipt.signature), receiptSigningBytes(tampered)), false);
  assert.equal(toHex(fromHex(PUBLIC_KEY)), PUBLIC_KEY);
});

test("a bad seed is refused", async () => {
  await assert.rejects(Signer.fromSeedHex("zz"));
  await assert.rejects(Signer.fromSeedHex("07".repeat(31)));
});
