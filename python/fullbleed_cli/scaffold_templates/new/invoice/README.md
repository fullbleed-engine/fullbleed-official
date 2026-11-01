# Invoice starter

Run `python report.py`, then open `output/invoice.pdf`. The starter includes
fictional sample data and uses the Inter font bundled with Fullbleed. It needs
no template engine or additional Python package.

- Edit `data.json` for the company, customer, dates, payment note, and line items.
- Edit `templates/invoice.html` for the document and `templates/item.html` for
  each line item. These use Python's `string.Template` syntax: `${customer_name}`
  inserts a value, and `$$` produces a literal dollar sign. Text from the JSON is
  HTML-escaped; only the renderer creates the row markup.
- Edit `templates/invoice.css` for layout, colors, and typography.
- Run `python report.py` again. PDF previews are in `output/preview`, and
  `output/invoice_run_report.json` records the calculated amounts and artifacts.

Use decimal strings for quantities, unit prices, and `tax_rate`. Each line
amount is rounded to two decimal places using decimal half-up rounding. The
subtotal is the sum of those rounded lines; tax is rounded once on the subtotal.
For example, `"tax_rate": "0.075"` means 7.5%. The sample uses zero tax. Adapt
these calculation rules in `report.py` to your application's invoice rules.

You can run the script from another working directory; inputs and outputs are
resolved relative to `report.py`. The JSON does not execute code. Adding an
unknown template placeholder raises an error instead of silently dropping it.
