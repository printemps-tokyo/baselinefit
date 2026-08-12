// Application entry point for the demo site.
//
// Three polyfills, one positioning library, and a handful of platform APIs at
// different points on the Baseline curve.

import ResizeObserverPolyfill from 'resize-observer-polyfill';
import smoothscroll from 'smoothscroll-polyfill';
import 'js-focus-visible';
import { computePosition, flip } from '@floating-ui/dom';

smoothscroll.polyfill();

const Observer = window.ResizeObserver || ResizeObserverPolyfill;

const cards = new Observer((entries) => {
  for (const entry of entries) {
    entry.target.dataset.width = String(entry.contentRect.width);
  }
});

document.querySelectorAll('.card').forEach((card) => cards.observe(card));

// Structured cloning of the initial state.
const initialState = structuredClone(window.__STATE__ ?? {});

// Route matching.
const route = new URLPattern({ pathname: '/cards/:id' });

// A background worker shared between tabs.
const worker = new SharedWorker('/scripts/sync-worker.js');
worker.port.start();

// Trusted Types, where the browser has them.
if (window.trustedTypes) {
  window.trustedTypes.createPolicy('default', {
    createHTML: (input) => input,
  });
}

export function openTooltip(anchor, bubble) {
  computePosition(anchor, bubble, { middleware: [flip()] }).then(({ x, y }) => {
    Object.assign(bubble.style, { left: `${x}px`, top: `${y}px` });
  });
}

export function openDialog(dialog) {
  dialog.showPopover();
}

export { initialState, route };
