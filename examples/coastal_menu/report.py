from __future__ import annotations

from pathlib import Path
import json

import fullbleed

from components.center_panel import CenterPanel, MenuItem, MenuSection
from components.fb_ui import Document, compile_document, el
from components.left_panel import ContactRow, LeftPanel
from components.right_panel import BeverageItem, RightPanel
from pipeline.validation import validate_render
from styles.tokens import render_root_vars

ROOT = Path(__file__).resolve().parent
OUTPUT_DIR = ROOT / "output"
CSS_PATH = ROOT / "styles" / "coastal_menu.css"
PDF_PATH = OUTPUT_DIR / "coastal_menu.pdf"
PNG_PATH = OUTPUT_DIR / "coastal_menu_page1.png"
SOURCE_IMAGE = ROOT / "costal_menu.png"
VALIDATION_PATH = OUTPUT_DIR / "coastal_menu_validation.json"


def create_engine() -> fullbleed.PdfEngine:
    bundle = fullbleed.AssetBundle()
    bundle.add_file(str(ROOT / "vendor/css/bootstrap.min.css"), "css", name="bootstrap")
    bundle.add_file(str(ROOT / "vendor/fonts/Inter-Variable.ttf"), "font", name="inter")
    bundle.add_file(str(ROOT / "vendor/icons/bootstrap-icons.svg"), "svg", name="bootstrap-icons")

    engine = fullbleed.PdfEngine(
        page_width="17in",
        page_height="8.5in",
        margin="0in",
        pdf_version="1.7",
    )
    engine.register_bundle(bundle)
    return engine


def _left_rows() -> list[ContactRow]:
    return [
        ContactRow(icon="location", lines=("123 Coastal Highway", "Seaside, CA 94000")),
        ContactRow(
            icon="clock",
            lines=("Mon-Thu: 5:00 PM - 10:00 PM", "Fri-Sat: 5:00 PM - 11:00 PM", "Sunday: 4:00 PM - 9:00 PM"),
        ),
        ContactRow(icon="phone", lines=("(555) 123-4567",)),
        ContactRow(icon="mail", lines=("reserve@coastaltable.com",)),
    ]


def _menu_sections() -> list[MenuSection]:
    return [
        MenuSection(
            title="Starters",
            items=(
                MenuItem(name="Oysters Rockefeller", price="$24", description="Fresh local oysters, spinach, pernod"),
                MenuItem(name="Tuna Tartare", price="$28", description="Yellowfin tuna, avocado, yuzu ponzu"),
                MenuItem(name="Lobster Bisque", price="$22", description="Maine lobster, cognac, creme fraiche"),
                MenuItem(name="Burrata & Tomatoes", price="$19", description="Creamy burrata, heirloom tomatoes"),
            ),
        ),
        MenuSection(
            title="Mains",
            items=(
                MenuItem(name="Mediterranean Sea Bass", price="$48", description="Whole roasted, lemon, seasonal vegetables"),
                MenuItem(name="Pan-Seared Scallops", price="$52", description="Cauliflower puree, pancetta, brown butter"),
                MenuItem(name="Surf & Turf", price="$72", description="Filet mignon, lobster tail, truffle potato"),
                MenuItem(name="Seafood Paella", price="$44", description="Saffron rice, prawns, mussels, chorizo"),
            ),
        ),
    ]


def _beverages() -> list[BeverageItem]:
    return [
        BeverageItem(name="Coastal Sunset", price="$18", description="Aperol, prosecco, blood orange"),
        BeverageItem(name="Ocean Breeze", price="$17", description="Gin, elderflower, cucumber"),
        BeverageItem(name="Beach Club Martini", price="$19", description="Vodka, lychee, rose"),
    ]


@Document(
    page="17x8.5-landscape",
    margin="0in",
    title="Coastal Menu Component Demo",
    bootstrap=True,
    root_class="report-root",
)
def App() -> object:
    return el(
        "div",
        LeftPanel(rows=_left_rows()),
        CenterPanel(sections=_menu_sections()),
        RightPanel(beverages=_beverages()),
        class_name="coastal-layout",
    )


def build_html() -> str:
    artifact = App()
    return compile_document(artifact)


def load_css() -> str:
    css_template = CSS_PATH.read_text(encoding="utf-8")
    token_block = render_root_vars()
    return css_template.replace("/* __TOKENS__ */", token_block, 1)


def _emit_preview_png(pdf_path: Path, png_path: Path) -> str:
    try:
        import fitz  # type: ignore
    except Exception:
        return "skipped (PyMuPDF missing)"
    doc = fitz.open(pdf_path)
    try:
        if doc.page_count == 0:
            return "skipped (empty PDF)"
        pix = doc[0].get_pixmap(dpi=144, alpha=False)
        pix.save(png_path)
    finally:
        doc.close()
    return "ok"


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    engine = create_engine()
    html = build_html()
    css = load_css()
    validation = validate_render(engine=engine, component=App, css=css, expected_page_count=1)
    VALIDATION_PATH.write_text(json.dumps(validation.to_dict(), indent=2), encoding="utf-8")

    bytes_written = engine.render_pdf_to_file(html, css, str(PDF_PATH))
    png_status = _emit_preview_png(PDF_PATH, PNG_PATH)

    print(f"[ok] Wrote {PDF_PATH} ({bytes_written} bytes)")
    print(f"[ok] Preview PNG: {PNG_PATH} ({png_status})")
    print(f"[ok] Validation: {VALIDATION_PATH} (ok={validation.ok})")
    print(f"[info] Source reference: {SOURCE_IMAGE}")


if __name__ == "__main__":
    main()
