# SPDX-License-Identifier: MIT
"""Authored content constraints and an adapter to the existing semantic rules.

These checks cover the component vocabulary's content boundaries. They are not
a complete HTML validator or a PDF accessibility conformance checker.
"""
from __future__ import annotations

from dataclasses import dataclass

from ._style import AuthoringError


_INLINE = frozenset("""
    a abbr b bdi bdo br button cite code data del dfn em i img input ins kbd label
    mark meter output picture progress q ruby s samp select small span strong sub
    sup svg textarea time u var wbr
""".split())
_INLINE_ONLY = frozenset("""
    p h1 h2 h3 h4 h5 h6 span strong em b i u s small code sub sup label abbr mark
    bdi bdo cite dfn kbd q samp time var
""".split())
_CHILDREN = {
    "table": frozenset({"caption", "colgroup", "thead", "tbody", "tfoot", "tr"}),
    "thead": frozenset({"tr"}), "tbody": frozenset({"tr"}), "tfoot": frozenset({"tr"}),
    "tr": frozenset({"th", "td"}), "colgroup": frozenset({"col"}),
    "ul": frozenset({"li"}), "ol": frozenset({"li"}),
}
_PARENTS = {
    "caption": frozenset({"table"}), "colgroup": frozenset({"table"}),
    "thead": frozenset({"table"}), "tbody": frozenset({"table"}), "tfoot": frozenset({"table"}),
    "tr": frozenset({"table", "thead", "tbody", "tfoot"}), "col": frozenset({"colgroup"}),
    "th": frozenset({"tr"}), "td": frozenset({"tr"}), "li": frozenset({"ul", "ol"}),
}


@dataclass(frozen=True)
class ContentContext:
    parent: str | None = "main"
    inline_owner: str | None = None
    inside_link: bool = False
    foreign: bool = False

    def text(self, value: str, path: tuple[str, ...]) -> None:
        if not self.foreign and self.parent in _CHILDREN and value.strip(" \t\r\n\f"):
            choices = ", ".join(sorted(_CHILDREN[self.parent]))
            raise AuthoringError(f"<{self.parent}> needs {choices} children, not bare text",
                                 code="structure.child", path=path)

    def element(self, tag: str, path: tuple[str, ...]) -> ContentContext:
        tag = tag.lower()
        if self.foreign:
            return ContentContext(tag, foreign=True)
        if self.inline_owner and tag not in _INLINE:
            raise AuthoringError(
                f"<{self.inline_owner}> needs inline content, but found <{tag}>. "
                "Place blocks beside paragraphs in a Flow or another block container.",
                code="structure.child", path=path,
            )
        if self.parent in _CHILDREN and tag not in _CHILDREN[self.parent]:
            choices = ", ".join(sorted(_CHILDREN[self.parent]))
            raise AuthoringError(f"<{self.parent}> needs {choices} children, but found <{tag}>",
                                 code="structure.child", path=path)
        if tag in _PARENTS and self.parent is not None and self.parent not in _PARENTS[tag]:
            choices = ", ".join(sorted(_PARENTS[tag]))
            raise AuthoringError(f"<{tag}> requires a {choices} parent, not <{self.parent}>",
                                 code="structure.parent", path=path)
        if tag == "a" and self.inside_link:
            raise AuthoringError("Links cannot contain another link", code="structure.link", path=path)
        return ContentContext(tag, self.inline_owner or (tag if tag in _INLINE_ONLY else None),
                              self.inside_link or tag == "a", tag == "svg")


def normalize_mode(value: str | None) -> str | None:
    mode = None if value is None else str(value).strip().lower()
    if mode in {None, "", "none"}:
        return None
    if mode not in {"warn", "raise"}:
        raise ValueError(f"Unsupported a11y_mode {value!r}. Expected None, 'warn', or 'raise'.")
    return mode


def semantic_report(root, document, sources, mode):
    from ..accessibility import A11yContract, _apply_validation_mode, _walk_elements
    from ..core import DocumentArtifact

    artifact = DocumentArtifact(root, page=str(document.page.size), margin=str(document.page.margin),
                                title=document.title, lang=document.lang, bootstrap=False)
    report = A11yContract().validate(artifact, mode=None)
    by_id = {source.node_id: source for source in sources}
    by_path = {path: by_id[node.props["data-fb-id"]]
               for node, path in _walk_elements(artifact)[0] if node.props.get("data-fb-id") in by_id}

    def location(source):
        return {"node_id": source.node_id, "path": list(source.path), "file": source.file, "line": source.line}

    for diagnostic in report["diagnostics"]:
        if source := by_path.get(diagnostic["path"]):
            diagnostic["source"] = location(source)
        related_paths = [diagnostic.get("first_seen_path"), *diagnostic.get("paths", ())]
        related = [location(by_path[path]) for path in related_paths if path in by_path]
        if related:
            diagnostic["related_sources"] = related
    report["mode"] = mode
    return _apply_validation_mode(report)
