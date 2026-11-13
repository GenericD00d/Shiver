import { expect, test } from 'bun:test';
import { seedKeyFor } from './seed-key';

test("seed keys match the core's, and differ per origin", async () => {
  // the same vector as seed_keys_are_per_origin in mobile/src-tauri/src/webview.rs
  expect(await seedKeyFor('secret', 'https://chat.example.com')).toBe('0ae7efbc5b8f6a54a87aef499c9c6d7593400a074e0257b2b0ec313b1ef0abde');
  expect(await seedKeyFor('secret', 'https://other.example.com')).not.toBe(await seedKeyFor('secret', 'https://chat.example.com'));
});
