from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path

import fitz


ROOT = Path(__file__).resolve().parents[1]
PDF_PATH = ROOT / "i-9.pdf"
LAYOUT_PATH = ROOT / "data" / "i9_field_layout.json"
DATA_PATH = ROOT / "data" / "data.json"
LEGACY_DATA_PATH = ROOT / "data" / "i9_data.json"

FIELD_FLAG_MULTILINE = 1 << 12
FIELD_FLAG_COMB = 1 << 24


MANUAL_KEYS: dict[tuple[int, str], str] = {
    (1, "CB_1"): "p01_section1_status_us_citizen",
    (1, "CB_2"): "p01_section1_status_noncitizen_national",
    (1, "CB_3"): "p01_section1_status_lawful_permanent_resident",
    (1, "CB_4"): "p01_section1_status_authorized_to_work_until",
    (1, "CB_Alt"): "p01_section2_alternative_procedure_used",
    (4, "CB_Alt_0"): "p04_reverification_row_1_alt_procedure_used",
    (4, "CB_Alt_1"): "p04_reverification_row_2_alt_procedure_used",
    (4, "CB_Alt_2"): "p04_reverification_row_3_alt_procedure_used",
}

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


@dataclass
class FieldEntry:
    key: str
    pdf_field_name: str
    page: int
    widget_index: int
    field_type: str
    x_pt: float
    y_pt: float
    width_pt: float
    height_pt: float
    field_flags: int
    text_maxlen: int
    text_font: str
    text_fontsize: float
    comb: bool
    multiline: bool


def _slugify(raw: str) -> str:
    text = raw.strip().lower().replace("&", " and ")
    text = re.sub(r"[^a-z0-9]+", "_", text)
    text = re.sub(r"_+", "_", text).strip("_")
    if not text:
        return "field"
    if text[0].isdigit():
        text = f"f_{text}"
    return text


def _build_key(*, page: int, field_name: str, used: set[str]) -> str:
    key = MANUAL_KEYS.get((page, field_name))
    if not key:
        key = f"p{page:02d}_{_slugify(field_name)}"
    base = key
    suffix = 2
    while key in used:
        key = f"{base}_{suffix}"
        suffix += 1
    used.add(key)
    return key


def _auto_value(entry: FieldEntry) -> object:
    key = entry.key
    name = entry.pdf_field_name.lower()

    if entry.field_type == "CheckBox":
        return False

    if entry.field_type == "ComboBox":
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

    # Keep fallback short so it fits tiny fields.
    return f"V{entry.page}{entry.widget_index:02d}"


def build() -> None:
    if not PDF_PATH.exists():
        raise FileNotFoundError(f"source PDF not found: {PDF_PATH}")

    doc = fitz.open(PDF_PATH)
    try:
        page_meta: list[dict[str, object]] = []
        fields: list[FieldEntry] = []
        used_keys: set[str] = set()

        for page_index in range(doc.page_count):
            page_no = page_index + 1
            page = doc[page_index]
            page_meta.append(
                {
                    "page": page_no,
                    "width_pt": round(float(page.rect.width), 3),
                    "height_pt": round(float(page.rect.height), 3),
                }
            )

            widget = page.first_widget
            widget_index = 0
            while widget:
                rect = widget.rect
                field_name = widget.field_name or f"unnamed_{page_no}_{widget_index}"
                key = _build_key(page=page_no, field_name=field_name, used=used_keys)
                field_flags = int(getattr(widget, "field_flags", 0) or 0)
                text_maxlen = int(getattr(widget, "text_maxlen", 0) or 0)
                text_font = str(getattr(widget, "text_font", "") or "")
                text_fontsize = float(getattr(widget, "text_fontsize", 0.0) or 0.0)
                fields.append(
                    FieldEntry(
                        key=key,
                        pdf_field_name=field_name,
                        page=page_no,
                        widget_index=widget_index,
                        field_type=str(widget.field_type_string or "Text"),
                        x_pt=round(float(rect.x0), 3),
                        y_pt=round(float(rect.y0), 3),
                        width_pt=round(float(rect.width), 3),
                        height_pt=round(float(rect.height), 3),
                        field_flags=field_flags,
                        text_maxlen=text_maxlen,
                        text_font=text_font,
                        text_fontsize=round(text_fontsize, 3),
                        comb=bool(field_flags & FIELD_FLAG_COMB),
                        multiline=bool(field_flags & FIELD_FLAG_MULTILINE),
                    )
                )
                widget_index += 1
                widget = widget.next

        fields.sort(key=lambda item: (item.page, item.widget_index, item.key))

        data_values: dict[str, object] = {}
        for item in fields:
            manual = MANUAL_VALUES.get((item.page, item.pdf_field_name))
            if manual is not None:
                data_values[item.key] = manual
            else:
                data_values[item.key] = _auto_value(item)

        LAYOUT_PATH.parent.mkdir(parents=True, exist_ok=True)
        DATA_PATH.parent.mkdir(parents=True, exist_ok=True)

        layout_payload = {
            "schema": "fullbleed.i9_field_layout.v1",
            "source_pdf": PDF_PATH.name,
            "page_count": doc.page_count,
            "pages": page_meta,
            "fields": [item.__dict__ for item in fields],
        }
        LAYOUT_PATH.write_text(json.dumps(layout_payload, indent=2), encoding="utf-8")

        data_payload = {
            "schema": "fullbleed.i9_data.v1",
            "source_pdf": PDF_PATH.name,
            "values": data_values,
        }
        DATA_PATH.write_text(json.dumps(data_payload, indent=2), encoding="utf-8")
        LEGACY_DATA_PATH.write_text(json.dumps(data_payload, indent=2), encoding="utf-8")
    finally:
        doc.close()


if __name__ == "__main__":
    build()
    print(f"[ok] wrote {LAYOUT_PATH}")
    print(f"[ok] wrote {DATA_PATH}")
