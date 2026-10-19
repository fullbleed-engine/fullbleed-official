// SPDX-License-Identifier: MIT
"use strict";
const get = (id) => document.getElementById(id);
let shownRevision = null;
let stopped = false;

function download(id, label, revision, name) {
  const link = get(id);
  link.textContent = label;
  link.href = `revisions/${revision}/${name}`;
  link.setAttribute("download", name);
  link.removeAttribute("aria-disabled");
}

function show(value) {
  const result = value.result;
  const stale = value.phase !== "ready";
  get("state").className = `status ${value.phase}`;
  get("title").textContent = value.document === "invoice" ? "Northstar Studio" : "Common Ground";
  get("intro").textContent = value.authoring === "components"
    ? "Python components → PDF. Save a component, stylesheet or data file to see your changes."
    : "HTML/CSS → PDF. Save a template, stylesheet or data file to see your changes.";
  get("status").textContent = value.phase === "ready" ? "Preview up to date"
    : value.phase === "error" ? "This edit could not be rendered" : "Rebuilding preview…";
  get("detail").textContent = stale && result
    ? "Showing the last successful preview. Downloads contain that earlier version."
    : value.phase === "error" ? "Fix the error and save again. The watcher will retry automatically."
    : value.phase === "ready" ? "Save another edit to rebuild. Ctrl-C in the terminal stops this preview."
    : "Loading the document and registered fonts.";
  get("error-panel").hidden = value.phase !== "error";
  get("error-summary").textContent = value.error.split("\n").filter(Boolean).pop() || "Render failed";
  get("error").textContent = value.error;
  if (!result) return;
  download("pdf", stale ? "Download last PDF" : "Download PDF", value.revision, "document.pdf");
  if (value.authoring === "components") {
    download("source", stale ? "Last styled HTML" : "Styled HTML", value.revision, "document.styled.html");
    download("extra", stale ? "Last unstyled HTML" : "Unstyled HTML", value.revision, "document.unstyled.html");
  } else {
    download("source", stale ? "Last HTML source" : "HTML source", value.revision, "source.html");
    download("extra", stale ? "Last CSS source" : "CSS source", value.revision, "styles.css");
  }
  get("meta").textContent = `${result.page_count} ${result.page_count === 1 ? "page" : "pages"} · ${result.dpi} DPI · Fullbleed ${result.engine_version}`;
  if (shownRevision === value.revision) return;
  shownRevision = value.revision;
  const figures = [];
  for (let page = 1; page <= result.page_count; page++) {
    const figure = document.createElement("figure");
    const caption = document.createElement("figcaption");
    caption.textContent = `Page ${page} / ${result.page_count}`;
    const image = document.createElement("img");
    image.src = `revisions/${value.revision}/page-${page}.png`;
    image.alt = `${get("title").textContent}, PDF page ${page}`;
    figure.append(caption, image);
    figures.push(figure);
  }
  get("pages").replaceChildren(...figures);
}

async function poll() {
  if (stopped) return;
  try {
    const response = await fetch("status", { cache: "no-store" });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    show(await response.json());
  } catch (_) {
    get("state").className = "status error";
    get("status").textContent = "Preview server disconnected";
    get("detail").textContent = "The displayed pages are from the last render. Restart the command and open its new URL.";
    for (const id of ["pdf", "source", "extra"]) {
      get(id).removeAttribute("href");
      get(id).setAttribute("aria-disabled", "true");
    }
  } finally {
    if (!stopped) window.setTimeout(poll, 500);
  }
}
window.addEventListener("pagehide", () => { stopped = true; });
poll();
