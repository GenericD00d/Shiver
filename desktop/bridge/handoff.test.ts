import { expect, test } from 'bun:test';

import { createDmTracker } from './dm-tracker';
import { createHotkeyHandoff } from './hotkey-handoff';

const state = {
  ownUserId: 1,
  users: [
    { id: 1, name: 'Me' },
    { id: 2, name: 'Ana' },
    { id: 3, name: 'Bob' }
  ],
  channels: [
    { id: 10, name: 'DM - 1:2', isDm: true },
    { id: 11, name: 'DM - 3:1', isDm: true },
    { id: 20, name: 'general' }
  ]
};

test('the DM list is drawn once the page is ready, and handed over only when it changed', () => {
  const dms = createDmTracker('https://chat.example');

  dms.update(state);
  expect(dms.take()).toBeNull();

  dms.start();
  dms.update({ ...state });

  expect(dms.take()?.map((dm) => dm.name)).toEqual(['Ana', 'Bob']);
  expect(dms.take()).toBeNull();

  // a newer message moves its conversation up; an older one changes nothing
  expect(dms.noteMessage(11, 500)).toBe(true);
  expect(dms.take()?.map((dm) => [dm.name, dm.lastMessageAt])).toEqual([
    ['Bob', 500],
    ['Ana', null]
  ]);
  expect(dms.noteMessage(11, 400)).toBe(false);
  expect(dms.take()).toBeNull();

  // the same store again draws nothing new
  dms.closedChanged();
  expect(dms.take()).toBeNull();
});

test('shortcuts wait for the drain, and each set is answered once', async () => {
  const hotkeys = createHotkeyHandoff();

  // a fresh page hands over none, which clears what a reload left
  expect(hotkeys.take()).toEqual([]);
  expect(hotkeys.take()).toBeNull();

  const replaced = hotkeys.set([{ actionId: 'mute', combo: 'ctrl+KeyM' }]);
  const latest = hotkeys.set([{ actionId: 'deafen', combo: 'ctrl+KeyD' }]);

  expect(await replaced).toEqual([]);
  expect(hotkeys.take()).toEqual([{ actionId: 'deafen', combo: 'ctrl+KeyD' }]);

  hotkeys.answer([{ actionId: 'deafen', reason: 'taken' }]);
  expect(await latest).toEqual([{ actionId: 'deafen', reason: 'taken' }]);

  // one drained before the core answered the one before it: that one is answered with no errors
  const first = hotkeys.set([{ actionId: 'a', combo: 'F1' }]);

  hotkeys.take();

  const second = hotkeys.set([{ actionId: 'b', combo: 'F2' }]);

  hotkeys.take();
  expect(await first).toEqual([]);

  hotkeys.answer([]);
  expect(await second).toEqual([]);
});
