/**
 * The key a seed must carry for an origin: HMAC-SHA256 of the origin, keyed with the per-launch
 * secret. `seed_key_for` in `mobile/src-tauri/src/webview.rs` computes the same. Here, beside `session.ts`, so its test runs with bun (`mobile/` type-checks without bun's types).
 */
export const seedKeyFor = async (secret: string, origin: string): Promise<string> => {
  const text = new TextEncoder();
  const key = await crypto.subtle.importKey('raw', text.encode(secret), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  const mac = await crypto.subtle.sign('HMAC', key, text.encode(origin));

  return Array.from(new Uint8Array(mac), (byte) => byte.toString(16).padStart(2, '0')).join('');
};
