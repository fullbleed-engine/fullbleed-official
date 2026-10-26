"""Run with the release-description tooling installed, separately from runtime tests."""
from io import BytesIO
from pathlib import Path
import tarfile
import tempfile
import unittest
import zipfile

from tools.check_package_description import check_artifacts


class PackageDescriptionTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def artifact(self, markdown, *, version="1.0.0", sdist=False, name=None):
        payload = (f"Metadata-Version: 2.1\nName: fullbleed\nVersion: {version}\n"
                   f"Description-Content-Type: text/markdown; charset=UTF-8; variant=GFM\n\n{markdown}\n").encode()
        path = self.root / (name or f"fullbleed-{version}{'.tar.gz' if sdist else '-cp310-abi3-win_amd64.whl'}")
        if sdist:
            with tarfile.open(path, "w:gz") as archive:
                info = tarfile.TarInfo(f"fullbleed-{version}/PKG-INFO")
                info.size = len(payload)
                archive.addfile(info, BytesIO(payload))
        else:
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr(f"fullbleed-{version}.dist-info/METADATA", payload)
        return path

    def test_rejects_rendered_relative_links_and_images(self):
        path = self.artifact('[Guide](docs/guide.md)\n\n[Examples][source]\n\n'
                             '[source]: examples/components\n\n<img src="assets/preview.png">')
        report, _ = check_artifacts([path])
        self.assertFalse(report["ok"])
        self.assertEqual({flag["detail"] for flag in report["flags"]},
                         {"docs/guide.md", "examples/components", "assets/preview.png"})

    def test_preserves_unicode_code_and_matching_distribution_descriptions(self):
        markdown = ('# Résumé\n\n[Start](#résumé) · [Guide](https://docs.fullbleed.dev/)\n\n'
                    '![PDF](https://docs.fullbleed.dev/example.png)\n\n'
                    '`[literal](not-a-link)`\n\n```markdown\n[example](also-not-a-link)\n```')
        report, html = check_artifacts([self.artifact(markdown), self.artifact(markdown, sdist=True)], "1.0.0")
        self.assertTrue(report["ok"], report["flags"])
        self.assertEqual(len(report["artifacts"]), 2)
        self.assertTrue(all("Résumé" in rendered for rendered in html.values()))
        urls = {item["url"] for item in report["artifacts"][0]["urls"]}
        self.assertIn("https://docs.fullbleed.dev/", urls)
        self.assertIn("https://docs.fullbleed.dev/example.png", urls)
        self.assertNotIn("not-a-link", urls)
        self.assertNotIn("also-not-a-link", urls)

    def test_rejects_missing_same_page_anchor(self):
        report, _ = check_artifacts([self.artifact('# Start\n\n[Missing](#gone)')])
        self.assertEqual([flag["code"] for flag in report["flags"]], ["DESCRIPTION_ANCHOR_MISSING"])

    def test_rejects_drift_between_wheel_and_sdist(self):
        report, _ = check_artifacts([self.artifact('# Current'), self.artifact('# Old', sdist=True)])
        self.assertIn("DESCRIPTION_SET_MISMATCH", {flag["code"] for flag in report["flags"]})

    def test_rejects_wrong_release_and_missing_metadata(self):
        valid = self.artifact('# Fullbleed')
        invalid = self.root / 'missing.whl'
        with zipfile.ZipFile(invalid, 'w') as archive:
            archive.writestr('unrelated.txt', 'not package metadata')
        report, _ = check_artifacts([valid, invalid], "1.0.1")
        self.assertEqual({flag["code"] for flag in report["flags"]},
                         {"DESCRIPTION_VERSION_MISMATCH", "DESCRIPTION_UNREADABLE"})


if __name__ == "__main__":
    unittest.main()
