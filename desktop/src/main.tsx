// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import React from "react";
import ReactDOM from "react-dom/client";

import { App } from "./App";
import "./styles.css";

/*
 * The webview's own context menu has no place in a desktop app - it offers
 * Reload, View Source and Inspect, none of which mean anything here, and it
 * appears instead of whatever menu the app wanted to show.
 *
 * Suppressed at the document, so a component only has to call
 * `preventDefault` where it wants its *own* menu. Inputs keep theirs, because
 * cut/copy/paste in a text field is genuinely useful and there is no custom
 * replacement for it.
 */
document.addEventListener("contextmenu", (event) => {
  const target = event.target as HTMLElement | null;
  if (target?.closest("input, textarea, [contenteditable='true'], .selectable")) return;
  event.preventDefault();
});

const root = document.getElementById("root");
if (!root) {
  throw new Error("index.html is missing #root");
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
