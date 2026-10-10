/** Telling the page whether the window is out of sight, which it cannot otherwise tell. */

let windowHidden = false;

let visibilityPatched = false;

/**
 * Makes `document.hidden` true while Shiver is minimised (a child webview stays "visible"), so
 * Sharkord notifies for messages in the channel on screen. Falls back to the real value otherwise.
 */
export function setWindowHidden(hidden: boolean) {
  if (!visibilityPatched) {
    const native = (name: 'hidden' | 'visibilityState') => Object.getOwnPropertyDescriptor(Document.prototype, name)?.get;
    const nativeHidden = native('hidden');
    const nativeState = native('visibilityState');

    try {
      Object.defineProperty(document, 'hidden', {
        configurable: true,
        get: () => windowHidden || (nativeHidden?.call(document) ?? false)
      });
      Object.defineProperty(document, 'visibilityState', {
        configurable: true,
        get: () => (windowHidden ? 'hidden' : (nativeState?.call(document) ?? 'visible'))
      });
      visibilityPatched = true;
    } catch {
      // sealed by the page
    }
  }

  if (windowHidden === hidden) return;

  windowHidden = hidden;
  document.dispatchEvent(new Event('visibilitychange'));
}
