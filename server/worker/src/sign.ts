// Ed25519 signing of start receipts and chunk acknowledgements.
//
// The byte layouts are fixed by the Rust verifier (crates/goalcup-protocol/src/receipt.rs and
// ack.rs) and reproduced here byte for byte; test/sign.test.ts checks them against signatures
// the Rust code produced. The signed bytes are
//
//   receipt: "goalcup-receipt-v1\0" ++ compact JSON of the claims in field order
//   ack:     "goalcup-ack-v1\0"     ++ {"entry_id":…,"seq":…,"hash":…,"acked_at":…}
//
// JSON.stringify writes no whitespace and keeps object keys in insertion order, and escapes
// strings the same way serde_json does for well-formed strings (", \, control characters;
// everything else, including non-ASCII, verbatim). So the objects below are built with their
// keys in exactly the verifier's order.

import { fromHex, toHex } from "./util.ts";

export interface ReceiptClaims {
  entry_id: string;
  connector_id: string;
  workspace: string;
  session_id: string;
  started_at: string;
  deadline_at: string;
  manifest_sha256: string;
  chain_head: string;
  connector_dir_sha256: string;
}

export interface StartReceipt {
  claims: ReceiptClaims;
  public_key: string;
  signature: string;
}

export interface Ack {
  entry_id: string;
  seq: number;
  hash: string;
  acked_at: string;
  server_signature: string;
}

const enc = new TextEncoder();
const RECEIPT_DOMAIN = "goalcup-receipt-v1\0";
const ACK_DOMAIN = "goalcup-ack-v1\0";

// Fixed 16-byte PKCS#8 header of an Ed25519 private key; the 32-byte seed follows.
const PKCS8_ED25519_HEADER = fromHex("302e020100300506032b657004220420");

export function receiptSigningBytes(c: ReceiptClaims): Uint8Array {
  // Explicit key order: this is the signed layout, do not "tidy" it.
  const ordered = {
    entry_id: c.entry_id,
    connector_id: c.connector_id,
    workspace: c.workspace,
    session_id: c.session_id,
    started_at: c.started_at,
    deadline_at: c.deadline_at,
    manifest_sha256: c.manifest_sha256,
    chain_head: c.chain_head,
    connector_dir_sha256: c.connector_dir_sha256,
  };
  return enc.encode(RECEIPT_DOMAIN + JSON.stringify(ordered));
}

export function ackSigningBytes(a: { entry_id: string; seq: number; hash: string; acked_at: string }): Uint8Array {
  const ordered = { entry_id: a.entry_id, seq: a.seq, hash: a.hash, acked_at: a.acked_at };
  return enc.encode(ACK_DOMAIN + JSON.stringify(ordered));
}

export class Signer {
  readonly publicKeyHex: string;
  readonly #privateKey: CryptoKey;

  private constructor(privateKey: CryptoKey, publicKeyHex: string) {
    this.#privateKey = privateKey;
    this.publicKeyHex = publicKeyHex;
  }

  /** `seedHex` is the 32-byte Ed25519 seed as 64 hex characters (the RECEIPT_SIGNING_KEY secret). */
  static async fromSeedHex(seedHex: string): Promise<Signer> {
    if (!/^[0-9a-fA-F]{64}$/.test(seedHex)) throw new Error("signing key must be 64 hex characters");
    const pkcs8 = new Uint8Array(PKCS8_ED25519_HEADER.length + 32);
    pkcs8.set(PKCS8_ED25519_HEADER, 0);
    pkcs8.set(fromHex(seedHex), PKCS8_ED25519_HEADER.length);
    // extractable: the public key is derived by exporting the JWK form (WebCrypto has no direct
    // "public key from private key" call); the private key itself is never exported.
    const jwkKey = await crypto.subtle.importKey("pkcs8", pkcs8, { name: "Ed25519" }, true, ["sign"]);
    const jwk = (await crypto.subtle.exportKey("jwk", jwkKey)) as JsonWebKey;
    if (typeof jwk.x !== "string") throw new Error("could not derive the public key");
    const publicKeyHex = toHex(base64UrlToBytes(jwk.x));
    const privateKey = await crypto.subtle.importKey("pkcs8", pkcs8, { name: "Ed25519" }, false, ["sign"]);
    return new Signer(privateKey, publicKeyHex);
  }

  async #sign(bytes: Uint8Array): Promise<string> {
    return toHex(new Uint8Array(await crypto.subtle.sign({ name: "Ed25519" }, this.#privateKey, bytes as BufferSource)));
  }

  async signReceipt(claims: ReceiptClaims): Promise<StartReceipt> {
    return { claims, public_key: this.publicKeyHex, signature: await this.#sign(receiptSigningBytes(claims)) };
  }

  async signAck(entry_id: string, seq: number, hash: string, acked_at: string): Promise<Ack> {
    const server_signature = await this.#sign(ackSigningBytes({ entry_id, seq, hash, acked_at }));
    return { entry_id, seq, hash, acked_at, server_signature };
  }
}

function base64UrlToBytes(s: string): Uint8Array {
  const b64 = s.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(s.length / 4) * 4, "=");
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
