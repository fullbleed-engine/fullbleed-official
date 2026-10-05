"""Rebuild the shared static Inter instance, outside all measurements."""
from pathlib import Path

from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

ROOT = Path(__file__).resolve().parent


def main():
    source = TTFont(ROOT / "fonts/Inter-Variable.ttf", recalcTimestamp=False)
    instance = instantiateVariableFont(source, {"opsz": 14, "wght": 400}, inplace=False)
    instance.recalcTimestamp = False
    instance.save(ROOT / "fonts/Inter-Regular.ttf")


if __name__ == "__main__":
    main()
