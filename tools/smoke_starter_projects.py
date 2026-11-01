"""Run packaged starter commands and inspect their actual document outputs."""
from __future__ import annotations

import argparse
from hashlib import sha256
from importlib.metadata import version
import json
from pathlib import Path
import subprocess
import sys

import fullbleed


def compact(text: str) -> str:
    return "".join(text.split())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=Path("target/starter-projects"))
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    workspace = out / "projects with spaces"
    workspace.mkdir(exist_ok=False)
    checks = []

    def run(label, argv, cwd):
        result = subprocess.run([sys.executable, *map(str, argv)], cwd=cwd, capture_output=True,
                                encoding="utf-8", timeout=180)
        (cwd / (label + ".stdout.txt")).write_text(result.stdout, encoding="utf-8")
        (cwd / (label + ".stderr.txt")).write_text(result.stderr, encoding="utf-8")
        assert result.returncode == 0, (label, cwd, result.returncode, result.stderr[-2000:])
        return result

    def create(template, name):
        project = workspace / name
        result = run(name + "-create", ["-I", "-m", "fullbleed", "new", "local", template, project, "--json"], workspace)
        assert json.loads(result.stdout)["ok"]
        return project

    def render(project, stem, markers, minimum_pages=1, default_data=False):
        # Execute outside the generated directory as well as using a path with
        # spaces. Project inputs must not depend on the caller's working directory.
        run(project.name + "-render", ["-I", project / "report.py"], workspace)
        pdf = project / "output" / (stem + ".pdf")
        report = json.loads((project / "output" / (stem + "_run_report.json")).read_bytes())
        assert report["ok"], (project, report)
        inspection = fullbleed.inspect_pdf(str(pdf))
        assert inspection["ok"] and inspection["page_count"] >= minimum_pages, inspection
        assert inspection["profile"]["embedded_font_count"] > 0, inspection
        extracted = fullbleed.extract_pdf_page_texts(str(pdf))
        assert extracted["ok"] and len(extracted["pages"]) == inspection["page_count"]
        text = "\n".join(page["text"] for page in extracted["pages"])
        for marker in markers:
            assert compact(marker) in compact(text), (project.name, marker, text)
        if default_data:
            assert not any(token in text for token in ("{{", "{%", "${")), text
        preview_dir = project / "output/finalized-preview"
        previews = fullbleed.PdfEngine().render_finalized_pdf_image_pages_to_dir(str(pdf), str(preview_dir), 96, stem)
        assert len(previews) == inspection["page_count"]
        item = {"project": project.name, "pdf": pdf.relative_to(out).as_posix(),
                "pdf_sha256": sha256(pdf.read_bytes()).hexdigest(), "inspection": inspection,
                "text": extracted, "run_report": report,
                "previews": [{"path": Path(path).relative_to(out).as_posix(),
                              "sha256": sha256(Path(path).read_bytes()).hexdigest()} for path in previews]}
        checks.append(item)
        print(json.dumps({"project": project.name, "pages": inspection["page_count"], "ok": True}), flush=True)
        return report, text, inspection

    for template, stem, markers in [
        ("invoice", "invoice", ["Northstar Studio", "FB-1042", "Jordan Lee", "1,284.50"]),
        ("statement", "statement", ["Northstar Studio", "Jordan Lee", "DEMO-1042", "1,268.00"]),
        ("accessible", "accessibility_scaffold", ["Accessibility Scaffold", "Account Summary", "Transactions", "Signature Evidence"]),
    ]:
        project = create(template, template + "-default")
        _, _, inspection = render(project, stem, markers, default_data=True)
        if template == "accessible":
            profile = inspection["profile"]
            assert all(profile[key] for key in ("struct_tree_root_present", "mark_info_present", "lang_present")), profile
            assert not profile["seed_blockers"], profile

    invoice = create("invoice", "invoice-edited")
    data_path = invoice / "data.json"
    data = json.loads(data_path.read_bytes())
    data["customer_name"] = 'R&D <Studio> "East"'
    data["tax_rate"] = "0.075"
    data["items"] = [{"description": f"WORK-{index:03d} Design, review & production <proof>",
                      "quantity": "1.5", "unit_price": "19.99"} for index in range(1, 49)]
    data_path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    report, text, _ = render(invoice, "invoice", ['R&D <Studio> "East"', "WORK-001", "WORK-048", "1,547.48"], minimum_pages=2)
    assert report["calculated"] == {"item_count": 48, "currency": "USD", "subtotal": "1439.52", "tax": "107.96", "total": "1547.48"}
    for index in range(1, 49):
        assert compact(text).count(f"WORK-{index:03d}") == 1
    assert text.count("Total:") == 1
    emitted = (invoice / "output/invoice.html").read_text(encoding="utf-8")
    assert "&lt;Studio&gt;" in emitted and "&lt;proof&gt;" in emitted

    statement = create("statement", "statement-edited")
    data_path = statement / "data.json"
    data = json.loads(data_path.read_bytes())
    data["opening_balance"] = "1000.00"
    data["transactions"] = [{"date": "2026-11-15", "description": f"TX-{index:03d} Credit & debit <entry>",
                             "amount": "1.005" if index % 2 else "-0.005"} for index in range(1, 41)]
    data_path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    report, text, _ = render(statement, "statement", ["TX-001", "TX-040", "1,020.00", "Credit & debit <entry>"], minimum_pages=2)
    assert report["calculated"]["closing_balance"] == "1020.00"
    for index in range(1, 41):
        assert compact(text).count(f"TX-{index:03d}") == 1

    result = {"ok": True, "engine": version("fullbleed"), "python": sys.version, "platform": sys.platform,
              "checks": checks,
              "scope": "Installed CLI creation, execution, calculated amounts, escaped text, pagination, embedded fonts, finalized PDF previews, and internal structure checks. Not independent PDF/UA certification."}
    (out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"ok": True, "engine": result["engine"], "projects": len(checks)}))


if __name__ == "__main__":
    main()
