from .fb_ui import component
from .primitives import Badge, Card, Row, Stack, Text


@component
def Header(*, title: str, subtitle: str, report_id: str):
    return Card(
        Row(
            Stack(
                Text(title, tag="h1", class_name="fb-header-title"),
                Text(subtitle, tag="p", class_name="fb-header-subtitle"),
                class_name="fb-header-copy",
            ),
            Badge(
                report_id,
                class_name="fb-header-badge",
                data_fb_role="report-id",
            ),
            class_name="fb-header-row",
        ),
        tag="header",
        class_name="fb-header-card",
        data_fb_role="header",
    )
