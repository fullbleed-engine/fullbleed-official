from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
PDF_PATH = ROOT / "i-9.pdf"
LAYOUT_PATH = ROOT / "data" / "i9_field_layout.json"
DATA_PATH = ROOT / "data" / "data.json"
LEGACY_DATA_PATH = ROOT / "data" / "i9_data.json"


MANUAL_VALUES: dict[tuple[int, str], object] = {
    (1, "Last Name (Family Name)"): "DOE",
    (1, "First Name Given Name"): "JANE",
    (1, "Employee Middle Initial (if any)"): "A",
    (1, "Employee Other Last Names Used (if any)"): "SMITH",
    (1, "Address Street Number and Name"): "123 MAIN ST",
    (1, "Apt Number (if any)"): "4B",
    (1, "City or Town"): "AUSTIN",
    (1, "State"): "TX",
    (1, "ZIP Code"): "78701",
    (1, "Date of Birth mmddyyyy"): "01/01/1990",
    (1, "US Social Security Number"): "123-45-6789",
    (1, "Employees E-mail Address"): "jane.doe@example.com",
    (1, "Telephone Number"): "(512) 555-0199",
    (1, "CB_1"): True,
    (1, "CB_2"): False,
    (1, "CB_3"): False,
    (1, "CB_4"): False,
    (1, "Signature of Employee"): "Jane Doe",
    (1, "Today's Date mmddyyy"): "02/17/2026",
    (1, "CB_Alt"): True,
    (1, "FirstDayEmployed mmddyyyy"): "02/18/2026",
    (1, "Last Name First Name and Title of Employer or Authorized Representative"): "ROE, RICHARD - HR MANAGER",
    (1, "Signature of Employer or AR"): "Richard Roe",
    (1, "S2 Todays Date mmddyyyy"): "02/17/2026",
    (1, "Employers Business or Org Name"): "FULLBLEED EXAMPLE INC",
    (1, "Employers Business or Org Address"): "500 MARKET ST, AUSTIN TX 78701",
    (3, "Last Name Family Name from Section 1"): "DOE",
    (3, "First Name Given Name from Section 1"): "JANE",
    (3, "Middle initial if any from Section 1"): "A",
    (3, "Preparer State 0"): "TX",
    (3, "Preparer State 1"): "TX",
    (3, "Preparer State 2"): "TX",
    (3, "Preparer State 3"): "TX",
    (4, "Last Name Family Name from Section 1-2"): "DOE",
    (4, "First Name Given Name from Section 1-2"): "JANE",
    (4, "Middle initial if any from Section 1-2"): "A",
    (4, "CB_Alt_0"): False,
    (4, "CB_Alt_1"): True,
    (4, "CB_Alt_2"): False,
}


def _slugify(raw: str) -> str:
    text = raw.strip().lower().replace("&", " and ")
    text = re.sub(r"[^a-z0-9]+", "_", text)
    text = re.sub(r"_+", "_", text).strip("_")
    if not text:
        return "field"
    if text[0].isdigit():
        text = f"f_{text}"
    return text


def _auto_value(field: dict[str, Any], fallback_key: str) -> object:
    name = str(field.get("pdf_field_name", "")).lower()
    field_type = str(field.get("field_type", "Text"))
    page = int(field.get("page", 0) or 0)
    widget_index = int(field.get("widget_index", 0) or 0)

    if field_type == "CheckBox":
        return False

    if field_type == "ComboBox":
        if "state" in name:
            return "TX"
        return "N/A"

    if "date" in name or "mmdd" in name:
        return "02/17/2026"
    if "zip" in name:
        return "10001"
    if "social" in name:
        return "123-45-6789"
    if "email" in name:
        return "employee@example.com"
    if "telephone" in name:
        return "(555) 010-0000"
    if "signature" in name:
        return "Jane Doe"
    if "city" in name:
        return "AUSTIN"
    if "state" in name:
        return "TX"
    if "name" in name:
        return "JANE DOE"
    if "address" in name:
        return "100 MAIN ST"

    return f"V{page}{widget_index:02d}-{_slugify(fallback_key)[:10]}"


def build() -> None:
    if not LAYOUT_PATH.exists():
        raise FileNotFoundError(
            f"layout file not found: {LAYOUT_PATH}\n"
            "This project now avoids third-party PDF parsers in tooling.\n"
            "Keep the checked-in layout JSON as canonical input for data seeding."
        )

    layout_payload = json.loads(LAYOUT_PATH.read_text(encoding="utf-8"))
    if not isinstance(layout_payload, dict):
        raise ValueError(f"invalid layout payload: {LAYOUT_PATH}")
    fields = layout_payload.get("fields")
    if not isinstance(fields, list) or not fields:
        raise ValueError(f"layout has no fields: {LAYOUT_PATH}")

    data_values: dict[str, object] = {}
    for idx, raw in enumerate(fields):
        if not isinstance(raw, dict):
            continue
        key = str(raw.get("key", "")).strip()
        if not key:
            key = f"field_{idx+1:04d}"
        page = int(raw.get("page", 0) or 0)
        pdf_field_name = str(raw.get("pdf_field_name", key))
        manual = MANUAL_VALUES.get((page, pdf_field_name))
        data_values[key] = manual if manual is not None else _auto_value(raw, key)

    DATA_PATH.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "schema": "fullbleed.i9_data.v1",
        "source_pdf": PDF_PATH.name,
        "values": data_values,
    }
    DATA_PATH.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    LEGACY_DATA_PATH.write_text(json.dumps(payload, indent=2), encoding="utf-8")


if __name__ == "__main__":
    build()
    print(f"[ok] refreshed {DATA_PATH}")
    print(f"[ok] refreshed {LEGACY_DATA_PATH}")
