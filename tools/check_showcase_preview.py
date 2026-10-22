"""Exercise the actual local preview server against a disposable source copy.

Requires the installed component alpha and the standard library. Source edits,
syntax errors, missing inputs, timeout recovery and both authoring paths cross
real process/HTTP boundaries. The original showcase source is never modified.
"""
from __future__ import annotations

import argparse
from hashlib import sha256
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


class Server:
    def __init__(self, project, output, document="invoice", authoring="components", timeout=15, python=None):
        self.project, self.output = project, output
        output.mkdir(parents=True)
        self.log = (output / "server.stdout").open("wb")
        self.errors = (output / "server.stderr").open("wb")
        self.process = subprocess.Popen(
            [str(python or sys.executable), str(project / "preview.py"), "--document", document,
             "--authoring", authoring, "--timeout", str(timeout)],
            cwd=project, stdin=subprocess.DEVNULL, stdout=self.log, stderr=self.errors,
            env={**os.environ, "PYTHONIOENCODING": "utf-8"},
        )
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            text = (output / "server.stdout").read_text(encoding="utf-8")
            if "\n" in text:
                self.url = json.loads(text.splitlines()[0])["url"]
                break
            if self.process.poll() is not None:
                raise AssertionError((output / "server.stderr").read_text(encoding="utf-8"))
            time.sleep(0.05)
        else:
            self.close()
            raise AssertionError("Preview server did not start")

    def close(self):
        if self.process.poll() is None:
            if os.name == "nt":
                result = subprocess.run(
                    ["taskkill", "/PID", str(self.process.pid), "/T", "/F"],
                    capture_output=True,
                )
                # The child can exit after poll() and before taskkill opens it.
                # Still fail if termination was rejected while it remains alive.
                if result.returncode and self.process.poll() is None:
                    result.check_returncode()
            else:
                self.process.send_signal(signal.SIGINT)
            self.process.wait(timeout=15)
            if os.name != "nt":
                assert self.process.returncode == 0, "Ctrl-C must stop the watcher cleanly"
        self.log.close()
        self.errors.close()

    def fetch(self, path, *, headers=None, expected=200):
        request = Request(self.url + path, headers=headers or {})
        try:
            response = urlopen(request, timeout=10)
        except HTTPError as error:
            response = error
        with response:
            assert response.status == expected, (path, response.status, response.read())
            return response.read(), dict(response.headers)

    def status(self):
        return json.loads(self.fetch("status")[0])

    def wait(self, phase="ready", after=0, timeout=45):
        deadline = time.monotonic() + timeout
        latest = None
        while time.monotonic() < deadline:
            assert self.process.poll() is None, "Preview process stopped"
            try:
                latest = self.status()
                if latest["phase"] == phase and latest["attempt"] > after:
                    return latest
            except URLError:
                pass
            time.sleep(0.1)
        raise AssertionError(f"Did not reach {phase!r} after attempt {after}: {latest}")

    def artifacts(self, label, state):
        folder = self.output / label
        folder.mkdir()
        (folder / "status.json").write_text(json.dumps(state, indent=2), encoding="utf-8")
        for name in state["result"]["files"]:
            data, headers = self.fetch(f"revisions/{state['revision']}/{name}")
            (folder / name).write_bytes(data)
            assert headers["Cache-Control"] == "no-store"
            assert headers["X-Content-Type-Options"] == "nosniff"
            if not name.endswith(".png"):
                assert headers["Content-Disposition"].startswith("attachment;")
        assert sha256((folder / "document.pdf").read_bytes()).hexdigest() == state["result"]["pdf_sha256"]
        return folder


def edit(path, old, new):
    text = path.read_text(encoding="utf-8")
    assert text.count(old) == 1, (path, old)
    path.write_text(text.replace(old, new), encoding="utf-8", newline="\n")


def pdf_text(path):
    import fullbleed
    result = fullbleed.extract_pdf_page_texts(str(path))
    return " ".join(" ".join(page["text"] for page in result["pages"]).split())


def check_components(project, out):
    server = Server(project, out)
    checks = []
    try:
        initial = server.wait()
        initial_files = server.artifacts("initial", initial)
        assert initial["result"]["page_count"] == 1
        assert b"<style" in (initial_files / "document.styled.html").read_bytes()
        assert b"<style" not in (initial_files / "document.unstyled.html").read_bytes()
        server.fetch("../status", expected=404)
        server.fetch("revisions/1/../../data.json", expected=404)
        server.fetch("status", headers={"Origin": "https://example.invalid"}, expected=403)
        server.fetch("status", headers={"Host": "example.invalid"}, expected=403)
        checks.append("loopback origin/host, token routes, artifact allowlist and attachment headers")

        # Component and imported-helper edits must not reuse stale pyc.
        component = project / "invoice_components.py"
        edit(component, 'Strong("Maple & Finch")', 'Strong("Maple & Cedar")')
        changed = server.wait(after=initial["attempt"])
        files = server.artifacts("component-edit", changed)
        assert "Maple & Cedar" in pdf_text(files / "document.pdf")
        assert changed["result"]["pdf_sha256"] != initial["result"]["pdf_sha256"]
        checks.append("fresh Python component source")

        helper = project / "component_primitives.py"
        edit(helper, 'f"${Decimal(value):,.2f}"', 'f"€{Decimal(value):,.2f}"')
        helpers = server.wait(after=changed["attempt"])
        files = server.artifacts("import-edit", helpers)
        assert "€1,870.00" in pdf_text(files / "document.pdf")
        checks.append("fresh imported component source")

        css = project / "invoice.css"
        css_before = css.read_text(encoding="utf-8")
        css.write_text(css_before + "\n.invoice .total { background: #b44123; }\n", encoding="utf-8")
        styled = server.wait(after=helpers["attempt"])
        files = server.artifacts("css-edit", styled)
        assert (files / "page-1.png").read_bytes() != (out / "import-edit/page-1.png").read_bytes()
        checks.append("CSS save refreshes saved-PDF pixels")

        data_path = project / "data.json"
        data_before = data_path.read_text(encoding="utf-8")
        data = json.loads(data_before)
        data["invoice"][0]["description"] = "Research & review <phase 1>"
        data_path.write_text(json.dumps(data), encoding="utf-8")
        data_changed = server.wait(after=styled["attempt"])
        files = server.artifacts("data-edit", data_changed)
        assert "Research & review <phase 1>" in pdf_text(files / "document.pdf")
        checks.append("JSON save and escaped content")

        good_source = component.read_text(encoding="utf-8")
        component.write_text(good_source + "\nthis is a syntax error !!!\n", encoding="utf-8")
        failed = server.wait("error", after=data_changed["attempt"])
        assert "SyntaxError" in failed["error"]
        assert failed["revision"] == data_changed["revision"]
        assert server.fetch(f"revisions/{failed['revision']}/document.pdf")[0] == (files / "document.pdf").read_bytes()
        server.artifacts("syntax-error-retained-preview", failed)
        component.write_text(good_source, encoding="utf-8")
        recovered = server.wait(after=failed["attempt"])
        assert recovered["result"]["pdf_sha256"] == data_changed["result"]["pdf_sha256"]
        checks.append("syntax error retains last good revision and recovers")

        # Rename emulates the temporary absence during an editor's atomic save.
        moved = project / "data.json.saved"
        assert project.resolve() in moved.resolve().parents
        data_path.rename(moved)
        missing = server.wait("error", after=recovered["attempt"])
        assert "FileNotFoundError" in missing["error"]
        moved.rename(data_path)
        restored = server.wait(after=missing["attempt"])
        assert restored["result"]["pdf_sha256"] == recovered["result"]["pdf_sha256"]
        checks.append("missing input stays watched and recovers")

        # Give one in-progress render time to observe an old input, then save
        # another input. The intermediate mixed build must not be published.
        component.write_text(good_source + "\nimport time\ntime.sleep(1.5)\n", encoding="utf-8")
        busy = server.wait("building", after=restored["attempt"])
        data["invoice"][0]["description"] = "Saved during rendering"
        data_path.write_text(json.dumps(data), encoding="utf-8")
        pending = server.wait(after=busy["attempt"])
        files = server.artifacts("edit-during-render", pending)
        assert pending["revision"] == restored["revision"] + 1
        assert "Saved during rendering" in pdf_text(files / "document.pdf")
        checks.append("save during a render queues a fresh rebuild without publishing mixed inputs")
        component.write_text(good_source, encoding="utf-8")
        settled = server.wait(after=pending["attempt"])
        attempt = settled["attempt"]
        time.sleep(0.7)
        assert server.status()["attempt"] == attempt
        checks.append("render artifacts do not trigger a rebuild loop")
    finally:
        server.close()
    return checks


def check_report(project, out, authoring):
    server = Server(project, out, "report", authoring)
    try:
        initial = server.wait()
        files = server.artifacts("initial", initial)
        assert initial["result"]["page_count"] == 3
        assert "What we count." in pdf_text(files / "document.pdf")
        if authoring == "components":
            component = project / "report_components.py"
            edit(component, "CoverPage(), ReachPage(report), ResourcesPage(report)", "CoverPage()")
        else:
            template = project / "report.html"
            text = template.read_text(encoding="utf-8")
            # Keep the complete first authored page and the document close.
            end = text.index('</section>') + len('</section>')
            template.write_text(text[:end] + '</main></body></html>', encoding="utf-8")
        reduced = server.wait(after=initial["attempt"])
        server.artifacts("one-page", reduced)
        assert reduced["result"]["page_count"] == 1
        assert "page-2.png" not in reduced["result"]["files"]
        server.fetch(f"revisions/{reduced['revision']}/page-2.png", expected=404)
        return [f"{authoring}: three-page report and removal of obsolete page artifacts"]
    finally:
        server.close()


def check_timeout(project, out):
    server = Server(project, out, timeout=5)
    try:
        initial = server.wait()
        component = project / "invoice_components.py"
        before = component.read_text(encoding="utf-8")
        component.write_text(before + "\nimport time\ntime.sleep(30)\n", encoding="utf-8")
        failed = server.wait("error", after=initial["attempt"])
        server.artifacts("timeout", failed)
        assert "exceeded 5 seconds" in failed["error"], failed
        assert failed["revision"] == initial["revision"]
        component.write_text(before, encoding="utf-8")
        recovered = server.wait(after=failed["attempt"])
        assert recovered["result"]["pdf_sha256"] == initial["result"]["pdf_sha256"]
        return ["hung render is terminated, last good preview retained, next save recovers"]
    finally:
        server.close()


def check_html_invoice(project, out):
    server = Server(project, out, authoring="html")
    try:
        initial = server.wait()
        server.artifacts("initial", initial)
        edit(project / "invoice.html", "Maple &amp; Finch", "Holly &amp; Pine")
        updated = server.wait(after=initial["attempt"])
        files = server.artifacts("edited", updated)
        assert "Holly & Pine" in pdf_text(files / "document.pdf")
        return ["HTML invoice source edit refreshes its PDF and separate HTML/CSS downloads"]
    finally:
        server.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--project", type=Path, default=Path(__file__).resolve().parents[1] / "examples/design_showcase")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    checks = []
    for name, callback in [("component-edits", check_components),
                           ("invoice-html", check_html_invoice),
                           ("report-components", lambda project, output: check_report(project, output, "components")),
                           ("report-html", lambda project, output: check_report(project, output, "html")),
                           ("timeout", check_timeout)]:
        project = out / name / "project"
        shutil.copytree(args.project, project, ignore=shutil.ignore_patterns("output", "__pycache__", "*.pyc"))
        checks.extend(callback(project, out / name / "evidence"))
        print(json.dumps({"case": name, "ok": True}), flush=True)
    matching_pages = []
    for document, left, right, count in [("invoice", "component-edits", "invoice-html", 1),
                                         ("report", "report-components", "report-html", 3)]:
        for page in range(1, count + 1):
            name = f"page-{page}.png"
            a = (out / left / "evidence/initial" / name).read_bytes()
            b = (out / right / "evidence/initial" / name).read_bytes()
            assert a == b, (document, page)
            matching_pages.append(dict(document=document, page=page, sha256=sha256(a).hexdigest()))
    result = dict(ok=True, checks=checks, python=sys.version, platform=sys.platform,
                  original_authoring_page_matches=matching_pages,
                  scope="Real local development server and edited synthetic showcase; not an engine conformance or delivery gate.")
    (out / "verification.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
