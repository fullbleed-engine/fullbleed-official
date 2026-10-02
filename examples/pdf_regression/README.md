# Catch PDF changes in pull requests

This standalone project renders the Northstar invoice and compares its inputs
and PDF bytes with a committed, reviewed baseline. It includes the HTML/CSS,
licensed fonts, baseline PDF, and a GitHub Actions workflow. The sample business,
contact details, and invoice data are fictional.

Use Python 3.10 or newer. From this directory:

```sh
python -m pip install -r requirements.txt
python check.py
```

The unchanged example should pass. Open `baseline.pdf` and
`output/invoice.pdf`; previews are in `output/preview/`, and the CLI result is
saved as `output/render.json`. The command also works from a different working
directory because the script resolves the project itself.

## Try a deliberate change

Change the invoice's heading color in `style.css`, then run `python check.py`.
The command exits with status 1. The result identifies input drift and a PDF hash
mismatch; the changed PDF and previews are still written. Neither `baseline.json`
nor `baseline.pdf` is replaced by a check.

Inspect both PDFs and every current preview. When the change is intentional:

```sh
python check.py record
python check.py
```

`record` renders again, then updates both baseline files only if that render
passes. Commit the source changes and both baseline files together. A check never
approves its own new output; the workflow only calls `python check.py`.

## Use it in a repository

Copy this entire directory, including `.github`, `.gitignore`, and
`.gitattributes`, into the root of a new repository. Push it to GitHub. The workflow
checks pushes to `main`/`master` and pull requests, and saves a `pdf-review` artifact
even when the document comparison fails. Download that artifact to compare the
baseline PDF with the changed PDF and previews.

For an existing application, adapt the workflow's working directory and dependency
installation to the document project. Add a required status check in your
repository's branch rules if you want failures to prevent merging.

## What the check proves

The engine version is pinned to 2.5.3 and fonts are local. Relative asset paths and
LF line endings keep the example's recorded inputs portable across checkouts.
Input-only changes, including comments or whitespace, can fail the fingerprint
check even when the PDF is unchanged. Engine upgrades and intentional data changes
should receive the same review as template changes.

This is an exact-byte regression check, not a perceptual image diff, accessibility
audit, or PDF standards validator. Its baseline must be reviewed by a person.
It is useful for fixed test data; do not put changing timestamps or live customer
data into the fixture.

The core runtime has no extra Python dependencies. Font source and license details
are in `fonts/font-sources.json` and the bundled OFL notices. The code and document
design use the MIT license in `LICENSE.txt`.
