# PDF invoice responses with FastAPI, Flask, and Django

Each app serves the same fictional invoice at
`http://127.0.0.1:8000/invoices/INV-1042.pdf`. Fullbleed renders the document into
bytes; the framework returns those bytes as an `application/pdf` attachment.
Unknown invoice IDs return HTTP 404.

The shared [invoice renderer](invoice.py) uses Python `Decimal` for the total,
escapes text for HTML, embeds the bundled Inter font, and creates an engine per
request. Rendering stays in memory, so concurrent requests do not share output
filenames. The endpoint uses a fixed download filename and disables response
caching.

## Run one app

From a repository checkout, enter this directory in a Python virtual environment:

```bash
cd examples/web_frameworks
```

Choose one framework. Only install its dependencies; these packages are separate
from the dependency-free Fullbleed runtime. The examples are checked with
Fullbleed 2.4.0 and the framework versions pinned in the requirement files.

### FastAPI

```bash
python -m pip install fullbleed -r requirements-fastapi.txt
python -m uvicorn fastapi_app:app --host 127.0.0.1 --port 8000
```

Open the PDF URL above. FastAPI's interactive API documentation is available at
`http://127.0.0.1:8000/docs` and declares the PDF response type.
The route uses a normal `def`, which FastAPI runs in its thread pool, so the
synchronous renderer is not called directly on the async event loop.
See [FastAPI's concurrency guide](https://fastapi.tiangolo.com/async/#path-operation-functions).

### Flask

```bash
python -m pip install fullbleed -r requirements-flask.txt
python -m flask --app flask_app run --host 127.0.0.1 --port 8000
```

### Django

```bash
python -m pip install fullbleed -r requirements-django.txt
python django_app.py runserver 127.0.0.1:8000 --noreload
```

This single-file Django example includes minimal local settings. In an existing
project, copy the view, URL pattern, and shared renderer into your own app and use
that project's settings.

## Use your application data

Replace `load_invoice()` with your application's authorized record lookup. Keep
access checks in that lookup or in the route before rendering. The helper expects
an invoice number, customer, issued/due date strings, and an item list containing
description, integer quantity, and unit-price strings. This small example omits
tax and discounts. Its record is fictional and requires no database.

These launch commands run local development servers. Use your framework's
deployment setup for a public application. For large jobs, move rendering to your
job queue and store the finished output; see the
[variable-data guide](https://docs.fullbleed.dev/guides/bank-statements/) for
compiled document families.

## Check all three examples

From the repository root, with a built Fullbleed wheel installed:

```bash
python -m pip install -r examples/web_frameworks/requirements-check.txt
python examples/web_frameworks/check_examples.py --out target/web-framework-check
```

The check uses each framework's test client, saves the returned PDFs, checks
headers, missing-record responses, expected text and total, embedded fonts, and
repeat-request bytes. It also checks FastAPI's OpenAPI media type and literal
markup-like customer text, then emits a PNG preview and `verification.json`.
CI runs it with the built wheel on Windows and Linux and retains the outputs.
