import { getCurrentWebview } from '@tauri-apps/api/webview';
import React from 'react';
import ReactDOM from 'react-dom/client';

import { App } from './App';
import { Bell } from './Bell';
import { NotificationsPopup } from './NotificationsPopup';
// the look both clients share, first, so each app's own styles can build on it
import '../../shared/web/components/ui.css';
import './styles.css';

/** One bundle serves the shell, the bell and the feed; the webview label picks which. */
const label = (() => {
  try {
    return getCurrentWebview().label;
  } catch {
    return new URLSearchParams(window.location.search).get('view') ?? 'shell';
  }
})();

const views: Record<string, () => React.ReactElement> = {
  overlay: Bell,
  popup: NotificationsPopup
};

const View = views[label] ?? App;

// A link or file dropped where nothing takes it would navigate this webview to it. Only the rail's
// own reorder targets take drops (they cancel the events themselves, before these run).
window.addEventListener('dragover', (event) => {
  if (event.defaultPrevented) return;

  event.preventDefault();

  if (event.dataTransfer) event.dataTransfer.dropEffect = 'none';
});
window.addEventListener('drop', (event) => event.preventDefault());

if (label === 'overlay') {
  // only the bell floats over a server page; the popup is an opaque panel
  document.documentElement.classList.add('overlay-root');
}

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <View />
  </React.StrictMode>
);
