"""Write an independent PDF for inspecting a viewer's link-border zoom behavior.

Uses only the Python standard library. Fullbleed does not generate this file.
"""
from argparse import ArgumentParser
from pathlib import Path


def make_pdf() -> bytes:
    # Both frames have a nominal 3pt border. The left is page artwork; the
    # right is a Link annotation without an appearance stream or NoZoom flag.
    stream = (
        b"0 0 0 RG 3 w 21.5 111.5 57 37 re S\n"
        b"BT /F1 9 Tf 20 164 Td (Page stroke: 3pt) Tj ET\n"
        b"BT /F1 9 Tf 110 164 Td (Link border: 3pt) Tj ET\n"
    )
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
        b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R /Annots [6 0 R] >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        b"<< /Length " + str(len(stream)).encode() + b" >>\nstream\n" + stream + b"endstream",
        b"<< /Type /Annot /Subtype /Link /Rect [110 110 170 150] /F 4 "
        b"/BS << /Type /Border /W 3 /S /S >> /C [0 0 0] "
        b"/A << /S /URI /URI (https://example.invalid/border-control) >> >>",
    ]
    pdf = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for number, obj in enumerate(objects, 1):
        offsets.append(len(pdf))
        pdf.extend(f"{number} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(pdf)
    pdf.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        pdf.extend(f"{offset:010d} 00000 n \n".encode())
    pdf.extend(
        f"trailer\n<< /Root 1 0 R /Size {len(objects) + 1} >>\n"
        f"startxref\n{xref}\n%%EOF\n".encode()
    )
    return bytes(pdf)


def main() -> None:
    parser = ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path, help="New PDF path; existing files are not overwritten.")
    args = parser.parse_args()
    with args.out.open("xb") as output:
        output.write(make_pdf())
    print(args.out)


if __name__ == "__main__":
    main()
