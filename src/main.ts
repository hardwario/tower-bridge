import { mount } from "svelte";
import "uplot/dist/uPlot.min.css";
import "./app.css";
import App from "./App.svelte";

// No browser context menu (Reload, Inspect); text fields keep theirs for copy and paste.
document.addEventListener("contextmenu", (e) => {
  const el = e.target as HTMLElement | null;
  if (!el?.closest("input, textarea, [contenteditable='true']")) e.preventDefault();
});

const target = document.getElementById("app");
if (!target) throw new Error("#app missing");

export default mount(App, { target });
