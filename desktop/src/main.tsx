import { getCurrentWebview } from '@tauri-apps/api/webview';
import React from 'react';
import ReactDOM from 'react-dom/client';

import { App } from './App';
import { Bell } from './Bell';
import { NotificationsPopup } from './NotificationsPopup';
import './styles.css';

/**
 * One bundle serves all three of Shiver's own webviews: the shell, the bell, and the notification
 * popup. The webview's own label decides which, rather than a query string, because
 * `WebviewUrl::App` takes a path and there is no guarantee a `?` survives being resolved against
 * the base url.
 */
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

if (label === 'overlay') {
  // only the bell floats over a server page; the popup is an opaque panel
  document.documentElement.classList.add('overlay-root');
}

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <View />
  </React.StrictMode>
);
