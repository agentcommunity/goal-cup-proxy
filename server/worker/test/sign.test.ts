// The Worker's signatures must match, byte for byte, the ones the Rust verifier's own code made.
// The vectors come from the Rust tests' key: Ed25519 seed = 32 bytes of 0x07
// (scratchpad/signing-vectors.txt, generated with crates/goalcup-protocol).

import assert from "node:assert/strict";
import { test } from "node:test";
import { Signer, ackSigningBytes, receiptSigningBytes, type ReceiptClaims } from "../src/sign.ts";
import { fromHex, toHex } from "../src/util.ts";

const SEED = "07".repeat(32);
const PUBLIC_KEY = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c";

const RUST_RECEIPT_CLAIMS: ReceiptClaims = {
  entry_id: "e0123456789abcdef",
  connector_id: "codex-cli",
  workspace: "/Users/p/goal-1/run-x/workspace — ünï",
  session_id: "6f1c2d3e-0000-4000-8000-000000000001",
  started_at: "2026-11-06T09:00:00Z",
  deadline_at: "2026-11-13T09:00:00Z",
  manifest_sha256: "a".repeat(64),
  chain_head: "b".repeat(64),
  connector_dir_sha256: "c".repeat(64),
};
const RUST_RECEIPT_SIGNATURE =
  "d3ffe33dbe7b663e63d246027245cc7540b458b1e33c9c888396b84745bc5bf2527621ea0068f7d015a8c22fe860590620735e1a5f8247c5c831e79bdc5c3f02";
const RUST_ACK_SIGNATURE =
  "5acf66735c55ef842f15a40f962e774eb26adb78d10d2c80942b06ea67b4eb7c6583b0bb7bc45ab169c6996e3b4f7cab19ccd17890a68cba1269c9dd980c440d";

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
  const ack = await signer.signAck("e0123456789abcdef", 812, "d".repeat(64), "2026-11-06T10:00:00Z");
  assert.deepEqual(ack, {
    entry_id: "e0123456789abcdef",
    seq: 812,
    hash: "d".repeat(64),
    acked_at: "2026-11-06T10:00:00Z",
    server_signature: RUST_ACK_SIGNATURE,
  });
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
