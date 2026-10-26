This test-only font is a renamed subset of BIZ UDPGothic Regular, licensed under
the accompanying SIL Open Font License. It covers ASCII and the synthetic CJK
text used in the table-wrapping tests. It is not a general-purpose Japanese font.

`font-source.json` records the pinned download, original and subset SHA-256
digests, and retained code points. To reproduce it, install FontTools 4.65.0,
download the recorded source, and run:

```sh
python tests/fixtures/table-wrapping/make_font.py /path/to/BIZUDPGothic-Regular.ttf
```

The test suite uses only the checked-in subset and makes no network requests.
