/**
 * Browser-only preview entry (`preview.html` → `vite preview`). Installs
 * the fixture IPC provider before the app mounts so the real components
 * can be exercised in a plain browser. The production entry (`main.ts`)
 * never imports this module.
 */
import { installPreviewIpc } from "./ipc-fixtures";
import { mount } from "svelte";
import App from "../src/App.svelte";

installPreviewIpc();

const target = document.getElementById("app");
if (!target) {
  throw new Error("#app mount point missing from preview.html");
}

export default mount(App, { target });
