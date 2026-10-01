# Rebuild PDFs as you edit

**Unreleased:** watch mode is available in the source checkout. It is not part
of the public 2.4.0 wheel. Follow the [contributor setup](../CONTRIBUTING.md#set-up-a-source-checkout)
to build and install this branch before running these commands.

Use `fullbleed render --watch` to render a document immediately, then rebuild it
after input changes. It uses Python's standard library and adds no runtime
dependencies.

## Try a complete invoice

From the repository root:

```sh
cd examples/render_watch
python -m fullbleed render --html invoice.html --css styles.css --asset "@noto-sans" --out invoice.pdf --emit-image preview --watch
```

Edit `styles.css`, save, and inspect `invoice.pdf` or its PNG preview. The
[sample source](../examples/render_watch) uses fictional data and a bundled
font. Fullbleed writes the preview files; it does not launch or refresh a viewer.
Press **Ctrl-C** to stop (exit code 130).

## Which changes trigger a rebuild?

Fullbleed polls the explicit HTML, CSS, and `--asset` paths. It also watches
file-backed page margins, PDF/VT jobs, template bindings and template maps, plus
local output-intent ICC, watermark, and reproducibility-check inputs. File-backed
JSON is read again on each render; inline strings remain unchanged.

Add dependencies that are referenced inside another file with repeatable
`--watch-path` options. Directories are watched recursively, including files
created after the process starts:

```sh
python -m fullbleed render --html invoice.html --css styles.css --out invoice.pdf --watch --watch-path assets --watch-path templates
```

Fullbleed does not discover every URL or path mentioned inside HTML/CSS or JSON.
For example, add a PDF template referenced by a template map as a watch path.
Changes to a JSON data file can trigger rendering, but watch mode does not run
your Python script to regenerate HTML. Regenerate the HTML in your application
and let Fullbleed watch that file.

The PDF, preview directory, and output files requested through CLI flags are
excluded from recursive scans, so a rebuild does not trigger itself. Common
cache/build directories such as `.git`, `.venv`, `__pycache__`, `node_modules`,
and `target` are skipped beneath recursive roots; nested directory symlinks are
not followed. Put redirected stdout or other application logs outside recursive
watch paths: the watcher cannot identify arbitrary logs as its own outputs.

## Saves, failures, and previews

The default polling interval is 0.5 seconds. Fullbleed waits for inputs to stay
unchanged for 0.2 seconds before rebuilding, coalescing a burst of editor saves.
Change these values with `--watch-interval` and `--watch-debounce` (seconds).
Edits made during a render trigger a subsequent render.

Missing files remain watched. A render or quality-gate failure is reported and
the process stays alive, ready for the next edit. Output follows the ordinary
render command: a failed quality gate may already have written a PDF, so a file
existing on disk does not prove that the render passed. Inspect the latest result.

When a successful rebuild reduces the page count, Fullbleed removes stale PNGs
from previous successful renders in that watch session only if they have not
been modified since it wrote them. It leaves unrelated or edited files alone.
Close a PDF viewer if it locks the output file on Windows, or use the PNG preview.

Watch mode requires file output and local inputs. It does not accept HTML/CSS
stdin or `--out -`. Inline HTML needs another local dependency or an explicit
watch path. Use a single watcher for each output path.

## Consume the result stream

```sh
python -m fullbleed --json-only render --html invoice.html --css styles.css --out invoice.pdf --watch
```

Stdout contains newline-delimited JSON using the existing
`fullbleed.render_result.v1` and `fullbleed.error.v1` schemas. There are no watch
status messages in JSON-only mode. Each successful render exposes its PDF
SHA-256 as `outputs.sha256`; `--emit-manifest` refreshes the manifest per cycle.

Use an ordinary one-shot `render` or `verify` invocation for a delivery or CI
gate. The watch process runs until interrupted and does not exit after a failed
render.
