/**
 * Editing a message on a phone. Sharkord edits in place, in a box that saves on Enter, which Shiver
 * makes a new line here as in the composer (`installReturnMakesALine`), so an edit could never be
 * finished. Instead, while an edit is open its box is drawn over the composer's own text area: the
 * composer's Send saves it, and a bar above says a message is being edited, with Cancel.
 *
 * React owns all of it, so nothing of Sharkord's is moved in the document: the edit box is fixed
 * over the composer (measured, and corrected where an ancestor makes `fixed` relative to itself),
 * the composer's text area is hidden under it, and the bar is Shiver's own.
 */

import { ensureStyle, hasLucideIcon, onDomSettled } from '../../shared/web/bridge/dom';
import { COMPOSE_EDITOR, MESSAGE_ITEM } from '../../shared/web/bridge/sharkord';

/** Sharkord's composer, and the row in it that holds its text area and buttons. */
const COMPOSER = '[class~="compose-container"]';
const COMPOSE_ROW = '[class~="compose-scroll-row"]';

/** the edit box while it is over the composer */
const EDIT_BOX = 'shiver-edit-box';
/** the message being edited, left in the list as a note */
const EDITED_ROW = 'shiver-edited-row';
/** the composer while an edit covers it */
const COVERED = 'shiver-composer-covered';
const EDIT_BAR = 'shiver-edit-bar';

const STYLES = `
.${EDIT_BOX} { position: fixed !important; z-index: 45; box-sizing: border-box; max-height: 40vh; overflow-y: auto; margin: 0 !important; }
/* Sharkord's "Press Enter to save": Send does that here */
.${EDIT_BOX} > span { display: none; }
.${EDITED_ROW}::after { content: 'Editing below'; display: block; font-size: 12px; font-style: italic; opacity: 0.6; }
.${COVERED} ${COMPOSE_ROW} > :first-child { visibility: hidden; }
/* what would land in the composer rather than the edit */
.${COVERED} ${COMPOSE_ROW} button:is(:has(svg.lucide-smile), :has(svg.lucide-smile-icon), :has(svg.lucide-paperclip), :has(svg.lucide-paperclip-icon)) { opacity: 0.4; pointer-events: none; }
.${EDIT_BAR} { position: fixed; z-index: 45; box-sizing: border-box; display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 6px 16px; font-size: 13px; border-top: 1px solid rgb(127 127 127 / 25%); }
.${EDIT_BAR} button { padding: 4px 10px; border: none; border-radius: 6px; background: rgb(127 127 127 / 20%); color: inherit; font: inherit; }
`;

type Editing = {
  row: HTMLElement;
  box: HTMLElement;
  composer: HTMLElement;
  bar: HTMLElement;
  stop: () => void;
};

let editing: Editing | null = null;

/** The edit open in the message list: its message and its box. */
function openEdit(): { row: HTMLElement; box: HTMLElement } | null {
  for (const editor of document.querySelectorAll<HTMLElement>(COMPOSE_EDITOR)) {
    const row = editor.closest<HTMLElement>(MESSAGE_ITEM);
    const box = row?.firstElementChild;

    if (row && box instanceof HTMLElement && box.contains(editor)) return { row, box };
  }

  return null;
}

/** The composer of the list `row` is in (a thread panel has its own): the nearest one up the tree. */
function composerOf(row: HTMLElement) {
  for (let node = row.parentElement; node; node = node.parentElement) {
    const composer = node.querySelector<HTMLElement>(COMPOSER);

    if (composer) return composer;
  }

  return null;
}

/**
 * Whether a computed colour is fully opaque. Painted to a pixel and read back, since Sharkord's are
 * `oklab()`/`oklch()` with a `/ alpha`, which no pattern over `rgba()` would read.
 */
function isOpaque(colour: string) {
  const context = document.createElement('canvas').getContext('2d', { willReadFrequently: true });

  if (!context) return false;

  context.clearRect(0, 0, 1, 1);
  context.fillStyle = colour;
  context.fillRect(0, 0, 1, 1);

  return context.getImageData(0, 0, 1, 1).data[3] === 255;
}

/** The first opaque background behind `element`, so the edit box hides what it covers. */
function backgroundBehind(element: HTMLElement) {
  for (let node: HTMLElement | null = element; node; node = node.parentElement) {
    const colour = getComputedStyle(node).backgroundColor;

    if (colour && isOpaque(colour)) return colour;
  }

  return '#0a0a0a';
}

function editorIn(box: HTMLElement) {
  return box.querySelector<HTMLElement>('.ProseMirror');
}

/** Sends `key` to the edit's editor, which saves on Enter and cancels on Escape. */
function press(box: HTMLElement, key: 'Enter' | 'Escape') {
  const editor = editorIn(box);

  if (!editor) return;

  editor.focus();
  // untrusted, so `installReturnMakesALine` lets it through as the Enter it is
  editor.dispatchEvent(new KeyboardEvent('keydown', { key, code: key, bubbles: true, cancelable: true }));
}

/** Puts the box over the composer's text area, and the bar above the box. */
function place({ box, composer, bar }: Editing) {
  const area = composer.querySelector<HTMLElement>(`${COMPOSE_ROW} > :first-child`) ?? composer;
  const target = area.getBoundingClientRect();
  const whole = composer.getBoundingClientRect();
  const fromBottom = window.innerHeight - target.bottom;

  box.style.left = `${target.left}px`;
  box.style.width = `${target.width}px`;
  box.style.minHeight = `${target.height}px`;
  box.style.top = 'auto';
  box.style.bottom = `${fromBottom}px`;

  // a transformed ancestor makes `fixed` relative to itself: measured, and moved by the difference
  const placed = box.getBoundingClientRect();
  const dx = placed.left - target.left;
  const dy = placed.bottom - target.bottom;

  if (Math.abs(dx) > 0.5 || Math.abs(dy) > 0.5) {
    box.style.left = `${target.left - dx}px`;
    box.style.bottom = `${fromBottom + dy}px`;
  }

  bar.style.left = `${whole.left}px`;
  bar.style.width = `${whole.width}px`;
  bar.style.bottom = `${window.innerHeight - box.getBoundingClientRect().top}px`;
}

function start(row: HTMLElement, box: HTMLElement, composer: HTMLElement): Editing {
  const background = backgroundBehind(composer);
  const bar = document.createElement('div');
  const label = document.createElement('span');
  const cancel = document.createElement('button');

  bar.className = EDIT_BAR;
  bar.style.background = background;
  bar.style.color = getComputedStyle(composer).color;
  label.textContent = 'Editing message';
  cancel.type = 'button';
  cancel.textContent = 'Cancel';
  // pressed without taking the focus, so the keyboard stays up
  cancel.addEventListener('pointerdown', (event) => event.preventDefault());
  cancel.addEventListener('click', () => press(box, 'Escape'));
  bar.append(label, cancel);
  document.body.appendChild(bar);

  box.style.background = background;

  const isSend = (target: EventTarget | null) => {
    const button = target instanceof Element ? target.closest('button') : null;

    return !!button && composer.contains(button) && (hasLucideIcon(button, 'send') || hasLucideIcon(button, 'send-horizontal'));
  };
  // the composer's Send saves the edit instead, and keeps the focus (and the keyboard) in it
  const onPress = (event: Event) => {
    if (isSend(event.target)) event.preventDefault();
  };
  const onClick = (event: Event) => {
    if (!isSend(event.target)) return;

    event.preventDefault();
    event.stopImmediatePropagation();
    press(box, 'Enter');
  };

  const current: Editing = { row, box, composer, bar, stop: () => undefined };
  const replace = () => place(current);
  const resized = new ResizeObserver(replace);

  document.addEventListener('pointerdown', onPress, true);
  document.addEventListener('click', onClick, true);
  window.addEventListener('resize', replace);
  window.visualViewport?.addEventListener('resize', replace);
  resized.observe(box);
  resized.observe(composer);

  current.stop = () => {
    document.removeEventListener('pointerdown', onPress, true);
    document.removeEventListener('click', onClick, true);
    window.removeEventListener('resize', replace);
    window.visualViewport?.removeEventListener('resize', replace);
    resized.disconnect();
    bar.remove();
    row.classList.remove(EDITED_ROW);
    box.classList.remove(EDIT_BOX);
    composer.classList.remove(COVERED);

    for (const property of ['left', 'width', 'min-height', 'top', 'bottom', 'background']) box.style.removeProperty(property);
  };

  return current;
}

/** Follows edits opening and closing, and keeps an open one over the composer. */
function check() {
  const open = openEdit();
  const composer = open ? composerOf(open.row) : null;

  if (editing && (!open || editing.box !== open.box || editing.composer !== composer)) {
    editing.stop();
    editing = null;
  }

  if (!open || !composer) return;

  editing ??= start(open.row, open.box, composer);

  // again each time: a render of Sharkord's that changes these class names drops Shiver's
  open.row.classList.add(EDITED_ROW);
  open.box.classList.add(EDIT_BOX);
  composer.classList.add(COVERED);
  place(editing);
}

export function installEditInComposer() {
  ensureStyle('shiver-edit').textContent = STYLES;
  onDomSettled(check);
}
