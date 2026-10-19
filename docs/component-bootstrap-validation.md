# Bootstrap authoring validation

Local qualification, 2026-10-16, Windows x64 / CPython 3.11. This records an
experimental component implementation, not a stable release or Bootstrap/PDF
conformance claim. Rust source and release version metadata are unchanged.

The final built and installed candidate wheel is
`fullbleed-2.5.26-cp310-abi3-win_amd64.whl`, SHA-256
`cf6eef1fca0dd7831179d89fce7fe5394c5a8a2e1c1b31ca05bfcbc5ce57e787`.
Its experimental Python modules and stubs match the working source. This wheel
is distinct from both PyPI 2.5.26 and the earlier `component-preview-f88cced`.

The first remote run passed on Linux but exposed Windows Git line-ending
conversion of the pinned Bootstrap stylesheet. The asset now has an explicit
byte-preserving `.gitattributes` rule. A checkout with `core.autocrlf=true`
reproduces the original checksum, and the existing asset test checks the
installed CSS bytes. The stylesheet and its integrity check remain unchanged.

## Checks

| Check | Result |
| --- | --- |
| Focused component/style/export tests | 198 passed |
| Full Python suite with the installed native extension | 860 passed |
| Bootstrap dependency | Exact upstream v5.0.0 CSS, SHA-256 `04083fcecdd9e19b2acd1e0073437aa07c98d230f0198d7f1fd470ab220fef64` |
| Native static catalog | All 23 component families plus layout, forms, content and utilities: 27 one-page specimens |
| Rich production report | One page; expected data retained; repeated PDF bytes identical |
| Long data table | 80 rows across 5 pages, each row once, headers on every page |
| Missing glyph reports | Empty for the report and all 27 catalog specimens |
| Independent PDF reading | pypdf checks contents, selection and hidden states |
| Independent raster reading | PDFium previews for report, buttons, forms and pagination; selected/unselected pagination fills asserted |
| Browser HTML | Headless Chromium 133.0.6943.16; all 27 groups; 1000px and 390px widths; no horizontal catalog overflow |
| Browser behavior | Correct authored tab/carousel/form states; in-flow static overlays; no scripts, page errors or external network requests |
| Unstyled HTML | No style elements, inline declarations, stylesheet links or scripts; authored values and hidden state remain |
| Installed typing | Mypy 2.4.0 rejects 50 deliberate errors; Pyright 1.1.414 rejects 51 and passes 21 completion checks |
| Packaging/contracts | Installed-package smoke, generated stubs, installed agent artifacts, version metadata and license integrity pass |
| Bundled Skill | `gh skill publish skills --dry-run` succeeds; nothing published |

Mypy's recursive iterable type still admits the existing bare-text `Flow` case;
runtime validation rejects it. Styling/token/part constraints that depend on
values remain runtime checks.

Reproduce native and browser evidence with
[`smoke_component_bootstrap.py`](../tools/smoke_component_bootstrap.py) and
[`smoke_component_bootstrap_browser.py`](../tools/smoke_component_bootstrap_browser.py).
The existing CI component job now runs the new tests and native catalog and
retains them under `target/component-recipes/bootstrap/`. Local raw evidence is
under `target/growth/bootstrap-authoring-20261016/`, including the wheel,
`installed/verification.json`, `installed/browser-verification.json`, PDF/PNG/HTML
artifacts, full-suite log, typing/editor output and the Skill validation result.

## What this establishes

The component vocabulary and pinned default CSS are available without handwritten
HTML. Resource collection is deterministic and document-local. Styled/unstyled
export is explicit, fragment roots can be inserted into lists/tables, and normal
document content checks remain intact. Group IDs, public card sections, state
validation and generated typing reduce coordination work for authors.

Native checks found missing input values, combined select options, unpainted
button backgrounds and list layout overriding navigation flex rules. Explicit
print representations now preserve values/selections, use static layout boxes,
retain real links, and capture numbered-list ordinals without depending on native
CSS counter scope across list layout boxes. A three-item pagination specimen also exposed a paint-order
interaction with Bootstrap's interactive z-index rules. Static print links use
document order; an independent raster assertion protects selected blue and
unselected white fills. These changes do not alter the native engine or claim
that arbitrary browser controls render identically in it.

## Limits

The catalog is a bounded set of specimens, not exhaustive testing of every
Bootstrap class combination, breakpoint, theme or pagination arrangement.
Interactive Bootstrap behaviors, Sass customization, browser pixel parity,
fillable PDF forms and accessibility conformance are not implemented or claimed.
PDF controls show authored state; no browser state or JavaScript is evaluated.
Fonts and image assets remain explicit application choices.

The deeper declared-part forwarding issue described in the
[implementation review](component-authoring-next-iteration.md) remains in the
pinned Bootstrap preview validated here. A subsequent source-branch update adds
explicit forwarding and distinguishes required from optional targets. The kit's
card sections continue to own their marked elements directly. Stable API
promotion, independent author feedback and other language surfaces remain
separate work.
