/**
 * Where the page starts: the stylesheets (bundled, never fetched), the
 * appearance onto <html> before the first paint, then React.
 */

import './styles/index.css';

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { applyAppearance } from './lib/settings';

applyAppearance();

const container = document.getElementById('root');
if (!container) throw new Error('#root is missing from index.html');

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
