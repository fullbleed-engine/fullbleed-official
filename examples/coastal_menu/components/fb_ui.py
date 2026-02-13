from __future__ import annotations

from dataclasses import dataclass, field
from html import escape
from typing import Any, Callable


@dataclass
class Element:
    tag: str
    props: dict[str, Any] = field(default_factory=dict)
    children: list[Any] = field(default_factory=list)


@dataclass
class DocumentArtifact:
    root: Element
    page: str
    margin: str
    title: str
    bootstrap: bool


def component(fn: Callable) -> Callable:
    """Marker decorator for function components."""
    fn.__fullbleed_component__ = True
    return fn


def el(tag: str, *children: Any, **props: Any) -> Element:
    flat: list[Any] = []
    for child in children:
        if child is None:
            continue
        if isinstance(child, (list, tuple)):
            flat.extend(x for x in child if x is not None)
        else:
            flat.append(child)
    return Element(tag=tag, props=props, children=flat)


def _normalize_attr_name(name: str) -> str:
    if name == "class_name":
        return "class"
    if name.startswith("data_fb_"):
        return "data-fb-" + name[len("data_fb_") :].replace("_", "-")
    return name.replace("_", "-")


def _render_attrs(props: dict[str, Any]) -> str:
    parts: list[str] = []
    for key, value in props.items():
        if value is None or value is False:
            continue
        attr = _normalize_attr_name(key)
        if value is True:
            parts.append(attr)
        else:
            parts.append(f'{attr}="{escape(str(value), quote=True)}"')
    return (" " + " ".join(parts)) if parts else ""


def render_node(node: Any) -> str:
    if node is None:
        return ""
    if isinstance(node, Element):
        attrs = _render_attrs(node.props)
        children_html = "".join(render_node(child) for child in node.children)
        return f"<{node.tag}{attrs}>{children_html}</{node.tag}>"
    return escape(str(node))


def Document(
    *,
    page: str = "LETTER",
    margin: str = "0.5in",
    title: str = "fullbleed document",
    bootstrap: bool = True,
    root_class: str = "report-root",
) -> Callable[[Callable[..., Any]], Callable[..., DocumentArtifact]]:
    def decorator(fn: Callable[..., Any]) -> Callable[..., DocumentArtifact]:
        def wrapped(*args: Any, **kwargs: Any) -> DocumentArtifact:
            tree = fn(*args, **kwargs)
            root = el(
                "main",
                tree,
                class_name=root_class,
                data_fb_role="document-root",
                data_fb_page=page,
            )
            return DocumentArtifact(
                root=root,
                page=page,
                margin=margin,
                title=title,
                bootstrap=bootstrap,
            )

        wrapped.__name__ = fn.__name__
        return wrapped

    return decorator


def compile_document(artifact: DocumentArtifact) -> str:
    return (
        "<!doctype html>"
        "<html lang=\"en\">"
        "<head>"
        "<meta charset=\"utf-8\" />"
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />"
        f"<title>{escape(artifact.title)}</title>"
        "</head>"
        "<body>"
        f"{render_node(artifact.root)}"
        "</body>"
        "</html>"
    )
