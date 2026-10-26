#!/usr/bin/env python
"""Render built distribution descriptions and check links before publishing.

Verification-only dependencies: readme-renderer[md]==45.0 and comrak==0.0.12.
Neither dependency is used by Fullbleed's build backend or installed runtime.
"""
from __future__ import annotations

import argparse
from email.parser import Parser
from hashlib import sha256
from html.parser import HTMLParser
from importlib.metadata import version
import json
from pathlib import Path
import tarfile
from urllib.parse import unquote, urlsplit
import zipfile


MAX_METADATA_BYTES = 5 * 1024 * 1024


class DescriptionHTML(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.urls: list[dict[str, str]] = []
        self.anchors: set[str] = set()

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attributes = dict(attrs)
        if attributes.get("id"):
            self.anchors.add(attributes["id"])
        if tag == "a" and attributes.get("name"):
            self.anchors.add(attributes["name"])
        attribute = {"a": "href", "img": "src"}.get(tag)
        if attribute and attributes.get(attribute):
            self.urls.append({"tag": tag, "attribute": attribute, "url": attributes[attribute]})


def metadata_bytes(path: Path) -> bytes:
    if path.suffix == ".whl":
        with zipfile.ZipFile(path) as archive:
            members = [item for item in archive.infolist()
                       if len(Path(item.filename).parts) == 2 and item.filename.endswith(".dist-info/METADATA")]
            if len(members) != 1 or members[0].file_size > MAX_METADATA_BYTES:
                raise ValueError("Expected exactly one bounded wheel METADATA file")
            return archive.read(members[0])
    if path.name.endswith(".tar.gz"):
        with tarfile.open(path, "r:gz") as archive:
            members = [item for item in archive.getmembers()
                       if item.isfile() and len(Path(item.name).parts) == 2 and Path(item.name).name == "PKG-INFO"]
            if len(members) != 1 or members[0].size > MAX_METADATA_BYTES:
                raise ValueError("Expected exactly one bounded source-distribution PKG-INFO file")
            stream = archive.extractfile(members[0])
            if stream is None:
                raise ValueError("Source-distribution metadata is unreadable")
            return stream.read(MAX_METADATA_BYTES + 1)
    raise ValueError("Expected a .whl or .tar.gz distribution")


def check_artifacts(paths: list[Path], expected_version: str | None = None) -> tuple[dict, dict[str, str]]:
    from readme_renderer.markdown import render

    flags: list[dict[str, str]] = []
    records: list[dict] = []
    rendered: dict[str, str] = {}

    def flag(code: str, artifact: str, detail: str) -> None:
        flags.append({"code": code, "artifact": artifact, "detail": detail})

    for path in paths:
        name = path.name
        try:
            raw = metadata_bytes(path)
            message = Parser().parsestr(raw.decode("utf-8"))
            description = message.get_payload()
            content_type = message.get("Description-Content-Type", "")
            if message.get("Name") != "fullbleed":
                raise ValueError("Expected the fullbleed distribution")
            if not isinstance(description, str) or not description.strip():
                raise ValueError("Package description is empty")
            if content_type.split(";", 1)[0].strip().lower() != "text/markdown":
                raise ValueError("Expected a Markdown package description")
            observed_version = message.get("Version")
            if not observed_version:
                raise ValueError("Package version is missing")
            if expected_version and observed_version != expected_version:
                flag("DESCRIPTION_VERSION_MISMATCH", name, f"Expected {expected_version}, observed {observed_version}")
            html = render(description)
            if not html:
                raise ValueError("PyPA's Markdown renderer did not produce HTML")
            rendered[name] = html
            page = DescriptionHTML()
            page.feed(html)
            for item in page.urls:
                url = item["url"]
                parsed = urlsplit(url)
                if url.startswith("#"):
                    if parsed.fragment and unquote(parsed.fragment) not in page.anchors:
                        flag("DESCRIPTION_ANCHOR_MISSING", name, url)
                elif parsed.scheme in {"http", "https"} and parsed.netloc:
                    continue
                elif parsed.scheme == "mailto" and parsed.path:
                    continue
                else:
                    flag("DESCRIPTION_URL_NOT_ABSOLUTE", name, url)
            records.append({
                "artifact": name, "name": message["Name"], "version": observed_version,
                "metadata_sha256": sha256(raw).hexdigest(),
                "description_sha256": sha256(description.encode("utf-8")).hexdigest(),
                "html_sha256": sha256(html.encode("utf-8")).hexdigest(),
                "content_type": content_type, "urls": page.urls,
            })
        except (OSError, ValueError, UnicodeError, zipfile.BadZipFile, tarfile.TarError) as error:
            flag("DESCRIPTION_UNREADABLE", name, str(error))
    if not paths:
        flag("DESCRIPTION_ARTIFACTS_MISSING", "", "No distributions supplied")
    if len({record["artifact"] for record in records}) != len(records):
        flag("DESCRIPTION_ARTIFACT_NAME_COLLISION", "", "Distribution filenames must be unique")
    identities = {(record["name"], record["version"], record["content_type"], record["description_sha256"])
                  for record in records}
    if len(identities) > 1:
        flag("DESCRIPTION_SET_MISMATCH", "", "Wheel and source-distribution descriptions must agree")
    report = {
        "schema": "fullbleed.package_description.v1", "ok": not flags,
        "renderer": {"readme-renderer": version("readme-renderer"), "comrak": version("comrak")},
        "artifacts": records, "flags": flags,
        "scope": "Rendered package descriptions, absolute link/image destinations, local anchors, and agreement across distributions. External target availability is checked separately.",
    }
    return report, rendered


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifacts", nargs="+", type=Path, help="Distribution files or directories containing them")
    parser.add_argument("--expected-version")
    parser.add_argument("--out", required=True, type=Path, help="Directory for JSON and PyPA-rendered HTML evidence")
    args = parser.parse_args()
    paths = []
    for path in args.artifacts:
        paths.extend(sorted([*path.glob("*.whl"), *path.glob("*.tar.gz")]) if path.is_dir() else [path])
    report, rendered = check_artifacts(paths, args.expected_version)
    args.out.mkdir(parents=True, exist_ok=True)
    report_path = args.out / "verification.json"
    report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    for name, html in rendered.items():
        (args.out / (name + ".html")).write_text(html, encoding="utf-8")
    print(json.dumps({"ok": report["ok"], "artifacts": len(paths), "flags": len(report["flags"]),
                      "report": str(report_path)}))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
