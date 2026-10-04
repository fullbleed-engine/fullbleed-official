# PDF invoice responses with FastAPI, Flask, and Django

Each app opens a small download page at `http://127.0.0.1:8000/` and serves the same fictional invoice at
`http://127.0.0.1:8000/invoices/INV-1042.pdf`. Fullbleed renders the document into
bytes; the framework returns those bytes as an `application/pdf` attachment.
Unknown invoice IDs return HTTP 404.

The shared [invoice renderer](invoice.py) uses Python `Decimal` for the total,
escapes text for HTML, embeds Inter plus the included DM Serif Display and Bebas Neue fonts, and creates an engine per
request. Rendering stays in memory, so concurrent requests do not share output
filenames. The endpoint uses a fixed download filename and disables response
caching.

## Run one app

Use Python 3.10–3.14. Extract the downloadable starter and open its
`fullbleed-python-invoice` directory, or use `examples/web_frameworks` in a
repository checkout. Create and activate a virtual environment first:

```bash
python -m venv .venv
```

Activate it on macOS or Linux:

```bash
source .venv/bin/activate
```

Or in Windows PowerShell:

```powershell
.venv\Scripts\Activate.ps1
```

Choose one framework. Only install its dependencies; these packages are separate
from the dependency-free Fullbleed runtime. The examples are checked with
Fullbleed 2.5.6 and the framework versions pinned in the requirement files.

### FastAPI

```bash
python -m pip install fullbleed==2.5.6 -r requirements-fastapi.txt
python -m uvicorn fastapi_app:app --host 127.0.0.1 --port 8000
```

Open `http://127.0.0.1:8000/` and select **Download invoice**. FastAPI's interactive API documentation is available at
`http://127.0.0.1:8000/docs` and declares the PDF response type.
The route uses a normal `def`, which FastAPI runs in its thread pool, so the
synchronous renderer is not called directly on the async event loop.
See [FastAPI's concurrency guide](https://fastapi.tiangolo.com/async/#path-operation-functions).

### Flask

```bash
python -m pip install fullbleed==2.5.6 -r requirements-flask.txt
python -m flask --app flask_app run --host 127.0.0.1 --port 8000
```

### Django

```bash
python -m pip install fullbleed==2.5.6 -r requirements-django.txt
python django_app.py runserver 127.0.0.1:8000 --noreload
```

This single-file Django example includes minimal local settings. In an existing
project, copy the view, URL pattern, and shared renderer into your own app and use
that project's settings.

## Use your application data

Edit `templates/invoice.html` for the document structure and
`templates/invoice.css` for its typography, columns, colors, and spacing.
The renderer reads these files for each download, so template edits appear
without restarting the server. The HTML uses Python `string.Template`
placeholders such as `$customer`, `$number`, `$rows`, and `$total`. A literal
dollar sign in the template is written as `$$`.

The included design is a fixed one-page A4 sample for three line items.
When adapting it for longer invoices, change the page geometry and pagination,
then inspect the resulting pages. This starter does not implement tax,
discounts, invoice numbering, payment collection, or application authentication.

Replace `load_invoice()` with your application's authorized record lookup. Keep
access checks in that lookup or in the route before rendering. The helper expects
an invoice number, customer, issued/due date strings, and an item list containing
description, an optional `detail`, integer quantity, and unit-price strings.
Its record is fictional and requires no database.

These launch commands run local development servers. Use your framework's
deployment setup for a public application. For large jobs, move rendering to your
job queue and store the finished output; see the
[variable-data guide](https://docs.fullbleed.dev/guides/bank-statements/) for
compiled document families.

## Check all three examples

From this directory, with Fullbleed installed:

```bash
python -m pip install -r requirements-check.txt
python check_examples.py --out output/check
```

The check uses each framework's test client and starts all three documented
local servers in turn. It checks real HTTP downloads, paired requests, headers,
404/405 responses, expected text and totals with an independent PDF reader,
embedded fonts, and matching bytes. Each server stops when its checks finish.
It also verifies FastAPI's OpenAPI media type, literal markup and placeholder-like
customer text, and the saved preview. CI runs it with a built wheel on Windows
and Linux and retains the outputs.

After editing the invoice, update the page's saved preview with:

```bash
python check_examples.py --out output/check --update-preview
```

The download page's screenshot is a saved sample; the PDF download is rendered
on request from the current template. Review `output/check/preview/invoice_page1.png`
and the PDF after a layout change.

## License

Example code and the Fullbleed engine are MIT licensed. The included display
fonts use SIL OFL 1.1; their licenses, pinned upstream sources, and hashes are in
`fonts/`. Inter and its license ship with the installed Fullbleed package.
