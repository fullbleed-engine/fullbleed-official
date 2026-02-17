from .fb_ui import component
from .primitives import Text


@component
def Footer(*, note: str):
    return Text(
        note,
        tag="footer",
        class_name="fb-footer",
        data_fb_role="footer",
    )
