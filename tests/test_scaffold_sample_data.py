from __future__ import annotations

from decimal import Decimal
import json
from pathlib import Path
import runpy
from string import Template
from types import SimpleNamespace

import pytest

from fullbleed_cli import scaffold


def project(template: str, root: Path):
    scaffold.cmd_new_template(SimpleNamespace(template=template, path=str(root), force=False, json=True))
    namespace = runpy.run_path(str(root / "report.py"))
    data = json.loads((root / "data.json").read_text(encoding="utf-8"), parse_float=Decimal)
    return namespace, data


def test_invoice_sample_has_calculated_total_and_no_template_instructions(tmp_path):
    namespace, data = project("invoice", tmp_path)
    context, calculated = namespace["invoice_context"](data)
    html = Template((tmp_path / "templates/invoice.html").read_text(encoding="utf-8")).substitute(context)
    assert calculated == {"item_count": 3, "currency": "USD", "subtotal": "1284.50", "tax": "0.00", "total": "1284.50"}
    for text in ("Northstar Studio", "Jordan Lee", "FB-1042", "1,284.50", "Printed proofs"):
        assert text in html
    assert not any(marker in html for marker in ("{{", "{%", "${"))


def test_invoice_rounds_lines_then_tax_and_preserves_unit_price_precision(tmp_path):
    namespace, data = project("invoice", tmp_path)
    data["items"] = [{"description": "Proofs", "quantity": "3", "unit_price": "0.005"},
                     {"description": "Layout", "quantity": "1.5", "unit_price": "19.99"}]
    data["tax_rate"] = "0.075"
    context, calculated = namespace["invoice_context"](data)
    assert calculated["subtotal"] == "30.01"
    assert calculated["tax"] == "2.25"
    assert calculated["total"] == "32.26"
    assert "0.005" in context["rows"]
    assert context["tax_percent"] == "7.5"


def test_invoice_escapes_customer_and_row_text_without_reprocessing_placeholders(tmp_path):
    namespace, data = project("invoice", tmp_path)
    data["customer_name"] = 'R&D <Studio> "East"'
    data["items"][0]["description"] = '<img src="remote"> ${untrusted} & review'
    context, _ = namespace["invoice_context"](data)
    assert context["customer_name"] == 'R&amp;D &lt;Studio&gt; &quot;East&quot;'
    assert '&lt;img src=&quot;remote&quot;&gt; ${untrusted} &amp; review' in context["rows"]
    assert '<img ' not in context["rows"]


@pytest.mark.parametrize("value", ["NaN", "Infinity", "-Infinity", "not a number"])
def test_invoice_rejects_nonfinite_or_invalid_amounts(tmp_path, value):
    namespace, data = project("invoice", tmp_path)
    data["items"][0]["unit_price"] = value
    with pytest.raises(ValueError, match="finite decimal"):
        namespace["invoice_context"](data)


def test_invoice_rejects_empty_items_zero_quantity_and_invalid_tax(tmp_path):
    namespace, data = project("invoice", tmp_path)
    with pytest.raises(ValueError, match="non-empty"):
        namespace["invoice_context"]({**data, "items": []})
    data["items"][0]["quantity"] = "0"
    with pytest.raises(ValueError, match="greater than zero"):
        namespace["invoice_context"](data)
    data["tax_rate"] = "7.5"
    with pytest.raises(ValueError, match="fraction"):
        namespace["invoice_context"](data)


def test_statement_sample_calculates_running_balances(tmp_path):
    namespace, data = project("statement", tmp_path)
    context, calculated = namespace["statement_context"](data)
    assert calculated == {"transaction_count": 3, "currency": "USD", "opening_balance": "740.00", "closing_balance": "1268.00"}
    assert all(balance in context["rows"] for balance in ("1,240.00", "1,228.00", "1,268.00"))
    assert "<td>12.00</td>" in context["rows"]
    html = Template((tmp_path / "templates/statement.html").read_text(encoding="utf-8")).substitute(context)
    assert "Jordan Lee" in html
    assert not any(marker in html for marker in ("{{", "{%", "${"))


def test_statement_rounds_signed_amounts_and_escapes_text(tmp_path):
    namespace, data = project("statement", tmp_path)
    data["opening_balance"] = "0"
    data["transactions"] = [{"date": "2026-11-01", "description": "R&D <credit>", "amount": "1.005"},
                            {"date": "2026-11-02", "description": "Fee", "amount": "-0.005"}]
    context, calculated = namespace["statement_context"](data)
    assert calculated["closing_balance"] == "1.00"
    assert "R&amp;D &lt;credit&gt;" in context["rows"]
    assert "1.01" in context["rows"]
    assert "0.01" in context["rows"]


def test_statement_empty_period_preserves_opening_balance(tmp_path):
    namespace, data = project("statement", tmp_path)
    data["transactions"] = []
    data["opening_balance"] = "123.455"
    context, calculated = namespace["statement_context"](data)
    assert calculated["opening_balance"] == calculated["closing_balance"] == "123.46"
    assert "No transactions in this period." in context["rows"]


@pytest.mark.parametrize("value", ["NaN", "Infinity", "bad amount"])
def test_statement_rejects_invalid_amounts(tmp_path, value):
    namespace, data = project("statement", tmp_path)
    data["transactions"][0]["amount"] = value
    with pytest.raises(ValueError, match="finite decimal"):
        namespace["statement_context"](data)
