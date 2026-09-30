import React from 'react';
import ReactDOM from 'react-dom/client';

import { App } from './App';
// the look both clients share, first, so each app's own styles can build on it
import '../../shared/web/components/ui.css';
import './styles.css';

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
