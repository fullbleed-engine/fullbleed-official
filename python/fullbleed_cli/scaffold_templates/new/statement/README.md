# Account statement starter

Run `python report.py`, then open `output/statement.pdf`. The sample account and
transactions are fictional. Fullbleed supplies the embedded Inter font; no
additional Python package or template engine is required.

- Edit `data.json` to change the account and transaction data. A positive
  `amount` is a credit; a negative amount is a debit. Transactions appear in
  their supplied order. The script calculates each running balance and the
  closing balance from the opening balance and those transactions.
- Edit `templates/statement.html` and `templates/transaction.html` to change the
  document and transaction row. They use Python's `string.Template` syntax,
  such as `${account_holder}`. Use `$$` for a literal dollar sign. JSON text is
  HTML-escaped before insertion; only the script creates row markup.
- Edit `templates/statement.css` to change the layout and colors.

Amounts are decimal strings, rounded to two decimal places with half-up
rounding before running balances are calculated. Adapt this rule in `report.py`
to match your source system. Empty transaction arrays are supported.

The script resolves paths relative to itself. You can rerun it from another
working directory. PNG previews are in `output/preview`; calculated balances
and artifact paths are in `output/statement_run_report.json`.
