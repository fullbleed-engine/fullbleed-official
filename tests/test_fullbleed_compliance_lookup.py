from pathlib import Path
from types import SimpleNamespace

from fullbleed_cli import cli


def distribution(root, files):
    return SimpleNamespace(files=files, locate_file=lambda entry: root / entry)


def test_distribution_license_wins_over_unrelated_ambient_license(tmp_path, monkeypatch):
    ambient = tmp_path / 'LICENSE'
    ambient.write_text('Unrelated Apache License', encoding='utf-8')
    relative = Path('fullbleed-2.3.1.dist-info/licenses/LICENSE')
    bundled = tmp_path / relative
    bundled.parent.mkdir(parents=True)
    bundled.write_text('MIT License', encoding='utf-8')
    monkeypatch.setattr(cli, '_compliance_roots', lambda: [tmp_path])
    monkeypatch.setattr(cli.metadata, 'distribution', lambda name: distribution(tmp_path, [relative]))
    assert cli._find_compliance_file('LICENSE') == bundled


def test_distribution_lookup_does_not_confuse_an_asset_license_with_package_license(tmp_path, monkeypatch):
    asset = Path('fullbleed_assets/fonts/LICENSE')
    package = Path('fullbleed-2.3.1.dist-info/licenses/LICENSE')
    for relative in (asset, package):
        target = tmp_path / relative
        target.parent.mkdir(parents=True)
        target.write_text(str(relative), encoding='utf-8')
    monkeypatch.setattr(cli, '_compliance_roots', lambda: [])
    monkeypatch.setattr(cli.metadata, 'distribution', lambda name: distribution(tmp_path, [asset, package]))
    assert cli._find_compliance_file('LICENSE') == tmp_path / package


def test_installed_module_never_treats_cwd_or_site_packages_as_source_root(tmp_path, monkeypatch):
    module = tmp_path / 'site-packages/fullbleed_cli/cli.py'
    module.parent.mkdir(parents=True)
    module.write_text('', encoding='utf-8')
    monkeypatch.setattr(cli, '__file__', str(module))
    monkeypatch.chdir(tmp_path)
    assert cli._compliance_roots() == []


def test_source_fallback_is_anchored_to_the_actual_cli_checkout(tmp_path, monkeypatch):
    module = tmp_path / 'checkout/python/fullbleed_cli/cli.py'
    module.parent.mkdir(parents=True)
    module.write_text('', encoding='utf-8')
    license_file = tmp_path / 'checkout/LICENSE'
    license_file.write_text('MIT License', encoding='utf-8')
    monkeypatch.setattr(cli, '__file__', str(module))
    monkeypatch.chdir(tmp_path)
    def absent(name):
        raise cli.metadata.PackageNotFoundError(name)
    monkeypatch.setattr(cli.metadata, 'distribution', absent)
    assert cli._compliance_roots() == [tmp_path / 'checkout']
    assert cli._find_compliance_file('LICENSE') == license_file
