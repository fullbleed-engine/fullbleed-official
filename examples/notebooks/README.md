# Try Fullbleed in a notebook

[Open in Google Colab](https://colab.research.google.com/github/fullbleed-engine/fullbleed-official/blob/master/examples/notebooks/first_invoice.ipynb)
or [view the notebook](first_invoice.ipynb).

Edit invoice data, render it with HTML/CSS and the bundled Inter font, inspect the
PDF, preview a page, and download the result. Colab needs a signed-in Google
account and a standard CPU runtime. This example does not need a paid plan or a
repository clone. You can also run it in a local Jupyter notebook with Python
3.10–3.14.

The notebook pins Fullbleed to the release tested for this example. Change the
data in step 2 and rerun steps 2–5 to regenerate the document. Runtime files in
Colab are temporary, so download the result before leaving.

## Maintain the example

Notebook tools remain separate from the Fullbleed runtime dependencies. Install
them only in a development environment:

```bash
python -m pip install -r examples/notebooks/requirements-check.txt
python tools/check_quickstart_notebook.py --out target/notebook-check
```

The check executes the actual notebook through a Jupyter kernel, saves an
executed copy and outputs under the selected directory, and verifies the default
invoice's page count, extracted text, embedded font, PNG preview, and byte
reproducibility. Review the PNG after changing layout. Keep committed notebook
outputs empty.

CI uses `--skip-install` to exercise the wheel it just built; the install cell is
the only skipped cell. Without that option the notebook's pinned install cell
runs too. Neither mode tests Google's hosted session or its download dialog.
