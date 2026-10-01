# SPDX-License-Identifier: MIT
from pathlib import Path

import fullbleed

html = "<h1>Invoice INV-1042</h1><p>Consulting: USD 1,200.00</p>"
css = "@page { size: A4; margin: 20mm; } h1 { color: #175c52; }"
pdf = fullbleed.PdfEngine().render_pdf(html, css)
Path("invoice.pdf").write_bytes(pdf)
