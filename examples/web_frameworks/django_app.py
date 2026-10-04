# SPDX-License-Identifier: MIT
"""Run: python django_app.py runserver 127.0.0.1:8000 --noreload"""

import os
import sys

from django.http import Http404, HttpResponse
from django.urls import path
from django.views.decorators.http import require_safe

from demo import DEMO_HEADERS, page_html, preview_bytes
from invoice import PDF_HEADERS, load_invoice, render_invoice


# Minimal local-demo settings. In an existing project, keep its settings and use
# the view and URL pattern below with that project's authorization checks.
SECRET_KEY = "fullbleed-local-example-not-a-production-secret"
DEBUG = False
ALLOWED_HOSTS = ["127.0.0.1", "localhost", "testserver"]
ROOT_URLCONF = "django_app"
MIDDLEWARE = []
INSTALLED_APPS = []


@require_safe
def index(request) -> HttpResponse:
    return HttpResponse(page_html(), content_type="text/html", headers=DEMO_HEADERS)


@require_safe
def preview(request) -> HttpResponse:
    return HttpResponse(preview_bytes(), content_type="image/png", headers=DEMO_HEADERS)


@require_safe
def invoice_pdf(request, invoice_id: str) -> HttpResponse:
    invoice = load_invoice(invoice_id)
    if invoice is None:
        raise Http404("Invoice not found")
    return HttpResponse(
        render_invoice(invoice), content_type="application/pdf", headers=PDF_HEADERS
    )


urlpatterns = [
    path("", index), path("preview.png", preview),
    path("invoices/<str:invoice_id>.pdf", invoice_pdf),
]


if __name__ == "__main__":
    os.environ.setdefault("DJANGO_SETTINGS_MODULE", "django_app")
    from django.core.management import execute_from_command_line

    execute_from_command_line(sys.argv)
