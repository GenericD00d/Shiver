import { beforeEach, expect, test } from 'bun:test';

/** A minimal DOM Storage, so the shim can patch `Storage.prototype` as it does in a page. */
class FakeStorage {
  data = new Map<string, string>();
  getItem(key: string) {
    return this.data.has(key) ? this.data.get(key)! : null;
  }
  setItem(key: string, value: string) {
    this.data.set(key, String(value));
  }
  removeItem(key: string) {
    this.data.delete(key);
  }
}

const g = globalThis as unknown as Record<string, unknown>;

let local: FakeStorage;
let session: FakeStorage;

beforeEach(async () => {
  local = new FakeStorage();
  session = new FakeStorage();
  // a fresh page each time: new Storage class and window
  class Storage extends FakeStorage {}
  Object.setPrototypeOf(local, Storage.prototype);
  Object.setPrototypeOf(session, Storage.prototype);
  g.Storage = Storage;
  g.window = { localStorage: local, sessionStorage: session, location: { hash: '', pathname: '/', search: '' } };
  g.history = { state: null, replaceState: (_: unknown, __: string, url: string) => ((g.window as any).location.hash = url.includes('#') ? url.slice(url.indexOf('#')) : '') };
});

const load = async () => (await import(`./session.ts?${Math.random()}`)) as typeof import('./session');

/** What Sharkord's auto-login controller does on load. */
const autoLogin = () => {
  if (local.getItem('sharkord-auto-login') !== 'true') return null;

  const saved = local.getItem('sharkord-auto-login-token');

  if (!saved) return null;

  session.setItem('sharkord-token', saved);

  return session.getItem('sharkord-token');
};

test('a seeded session signs in without anything reaching storage', async () => {
  const { installSessionShim } = await load();

  local.data.set('sharkord-auto-login-token', 'stale-copy-on-disk');
  installSessionShim('T');

  expect(autoLogin()).toBe('T');
  expect(local.data.size).toBe(0);
  expect(session.data.size).toBe(0);
});

test('a refused token hands back to storage and leaves nothing behind', async () => {
  const { installSessionShim } = await load();
  const shim = installSessionShim('T');

  autoLogin();
  // Sharkord's failure path
  local.removeItem('sharkord-auto-login-token');
  local.setItem('sharkord-auto-login', 'false');

  expect(shim.active()).toBe(false);
  expect(local.getItem('sharkord-auto-login')).toBe('false');
  expect(session.data.has('sharkord-token')).toBe(false);
});

test('signing in on the page afterwards works as if Shiver were not there', async () => {
  const { installSessionShim } = await load();

  installSessionShim('T');
  // the connect screen: live token, then the user's own choice
  session.setItem('sharkord-token', 'MINE');
  local.setItem('sharkord-auto-login', 'true');
  local.setItem('sharkord-auto-login-token', 'MINE');

  expect(session.getItem('sharkord-token')).toBe('MINE');
  expect(local.data.get('sharkord-auto-login-token')).toBe('MINE');
});

test('other keys are untouched and a second install only reseeds', async () => {
  const { installSessionShim } = await load();
  const first = installSessionShim(null);

  local.setItem('sharkord-auto-join-last-channel', 'true');
  expect(local.getItem('sharkord-auto-join-last-channel')).toBe('true');
  expect(local.getItem('sharkord-auto-login-token')).toBe(null);

  expect(installSessionShim('T')).toBe(first);
  expect(local.getItem('sharkord-auto-login-token')).toBe('T');
});

test('the seed is taken out of the fragment, keeping the rest', async () => {
  const { takeSeedFromLocation } = await load();

  (g.window as any).location.hash = '#oidc=1&shiver-seed=k.abc.def';
  expect(takeSeedFromLocation()).toEqual(['k', 'abc.def']);
  expect((g.window as any).location.hash).toBe('#oidc=1');
  expect(takeSeedFromLocation()).toBe(null);

  for (const hash of ['#shiver-seed=abc', '#shiver-seed=.abc', '#shiver-seed=k.']) {
    (g.window as any).location.hash = hash;
    expect(takeSeedFromLocation()).toBe(null);
    expect((g.window as any).location.hash).toBe('');
  }
});
