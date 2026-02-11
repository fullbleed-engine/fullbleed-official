# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
"""Project scaffolding commands for fullbleed.

Provides commands for initializing new projects and creating templates.
"""
import json
import os
import sys
from pathlib import Path
from datetime import datetime


# Default project structure
# Default project structure
DEFAULT_INIT_FILES = {
    "fullbleed.toml": '''[project]
name = "my-report"
version = "0.1.0"
entrypoint = "src/report.py:create_engine"

[build]
output = "dist/report.pdf"
html = "src/templates/index.html"
css = ["src/styles/main.css"]

[assets]
# "bootstrap" = { version = "5.3", kind = "css" }
# "inter" = { version = "4.0", kind = "font" }
''',
    "src/report.py": '''import fullbleed

def create_engine():
    """Create and configure the PDF engine."""
    engine = fullbleed.PdfEngine(
        page_width="8.5in",
        page_height="11in",
        margin="0.5in",
    )
    return engine
''',
    "src/templates/index.html": '''<!DOCTYPE html>
<html>
<body>
    <h1>Hello, Fullbleed!</h1>
    <p>This is a PDF generated from HTML.</p>
</body>
</html>
''',
    "src/styles/main.css": '''
@page { margin: 1in; }
body { font-family: sans-serif; }
h1 { color: #333; }
''',
    ".gitignore": '''# Fullbleed output
dist/
*.pdf

# Python
__pycache__/
*.pyc
.venv/
.pytest_cache/

# IDE
.vscode/
.idea/
''',
}

DEFAULT_DIRS = ["src", "src/templates", "src/styles", "dist"]


# Sample templates
TEMPLATES = {
    "invoice": {
        "description": "Basic invoice template",
        "files": {
            "templates/invoice.html": '''<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>Invoice</title>
</head>
<body>
    <header class="invoice-header">
        <div class="company-info">
            <h1>{{ company_name }}</h1>
            <p>{{ company_address }}</p>
        </div>
        <div class="invoice-info">
            <h2>INVOICE</h2>
            <p>Invoice #: {{ invoice_number }}</p>
            <p>Date: {{ invoice_date }}</p>
            <p>Due: {{ due_date }}</p>
        </div>
    </header>
    
    <section class="billing">
        <div class="bill-to">
            <strong>Bill To:</strong>
            <p>{{ customer_name }}</p>
            <p>{{ customer_address }}</p>
        </div>
    </section>
    
    <table class="line-items">
        <thead>
            <tr>
                <th>Description</th>
                <th>Qty</th>
                <th>Unit Price</th>
                <th>Amount</th>
            </tr>
        </thead>
        <tbody>
            {% for item in items %}
            <tr>
                <td>{{ item.description }}</td>
                <td>{{ item.quantity }}</td>
                <td>${{ item.unit_price }}</td>
                <td>${{ item.amount }}</td>
            </tr>
            {% endfor %}
        </tbody>
        <tfoot>
            <tr>
                <td colspan="3" class="total-label">Total:</td>
                <td class="total-amount">${{ total }}</td>
            </tr>
        </tfoot>
    </table>
    
    <footer>
        <p>Thank you for your business!</p>
    </footer>
</body>
</html>
''',
            "templates/invoice.css": '''/* Invoice Template Styles */
@page {
    size: letter;
    margin: 0.75in;
}

body {
    font-family: system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
    font-size: 11pt;
    line-height: 1.4;
    color: #333;
}

.invoice-header {
    display: flex;
    justify-content: space-between;
    margin-bottom: 2rem;
    padding-bottom: 1rem;
    border-bottom: 2px solid #2563eb;
}

.company-info h1 {
    margin: 0;
    font-size: 1.5rem;
    color: #1e40af;
}

.invoice-info {
    text-align: right;
}

.invoice-info h2 {
    margin: 0 0 0.5rem;
    font-size: 1.25rem;
    color: #1e40af;
}

.billing {
    margin-bottom: 2rem;
}

.line-items {
    width: 100%;
    border-collapse: collapse;
    margin-bottom: 2rem;
}

.line-items th,
.line-items td {
    padding: 0.75rem;
    text-align: left;
    border-bottom: 1px solid #e5e7eb;
}

.line-items th {
    background: #f3f4f6;
    font-weight: 600;
}

.line-items th:last-child,
.line-items td:last-child {
    text-align: right;
}

.total-label {
    text-align: right;
    font-weight: 600;
}

.total-amount {
    font-weight: 600;
    font-size: 1.1em;
    color: #1e40af;
}

footer {
    margin-top: 3rem;
    padding-top: 1rem;
    border-top: 1px solid #e5e7eb;
    text-align: center;
    font-size: 0.9em;
    color: #6b7280;
}
''',
        },
    },
    "statement": {
        "description": "Bank/account statement template",
        "files": {
            "templates/statement.html": '''<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>Account Statement</title>
</head>
<body>
    <header>
        <h1>{{ institution_name }}</h1>
        <p>Account Statement</p>
    </header>
    
    <section class="account-info">
        <p><strong>Account Holder:</strong> {{ account_holder }}</p>
        <p><strong>Account Number:</strong> {{ account_number }}</p>
        <p><strong>Statement Period:</strong> {{ start_date }} - {{ end_date }}</p>
    </section>
    
    <section class="summary">
        <div class="balance-box">
            <span class="label">Opening Balance</span>
            <span class="amount">${{ opening_balance }}</span>
        </div>
        <div class="balance-box">
            <span class="label">Closing Balance</span>
            <span class="amount">${{ closing_balance }}</span>
        </div>
    </section>
    
    <table class="transactions">
        <thead>
            <tr>
                <th>Date</th>
                <th>Description</th>
                <th>Debit</th>
                <th>Credit</th>
                <th>Balance</th>
            </tr>
        </thead>
        <tbody>
            {% for tx in transactions %}
            <tr>
                <td>{{ tx.date }}</td>
                <td>{{ tx.description }}</td>
                <td>{% if tx.debit %}${{ tx.debit }}{% endif %}</td>
                <td>{% if tx.credit %}${{ tx.credit }}{% endif %}</td>
                <td>${{ tx.balance }}</td>
            </tr>
            {% endfor %}
        </tbody>
    </table>
</body>
</html>
''',
            "templates/statement.css": '''/* Statement Template Styles */
@page {
    size: letter;
    margin: 0.5in;
}

body {
    font-family: "Segoe UI", Roboto, sans-serif;
    font-size: 10pt;
    line-height: 1.3;
}

header {
    text-align: center;
    margin-bottom: 1.5rem;
    padding-bottom: 1rem;
    border-bottom: 2px solid #1f2937;
}

header h1 {
    margin: 0;
    font-size: 1.5rem;
}

.account-info {
    margin-bottom: 1.5rem;
    padding: 1rem;
    background: #f9fafb;
    border-radius: 4px;
}

.account-info p {
    margin: 0.25rem 0;
}

.summary {
    display: flex;
    gap: 2rem;
    margin-bottom: 1.5rem;
}

.balance-box {
    flex: 1;
    padding: 1rem;
    background: #eff6ff;
    border-radius: 4px;
    text-align: center;
}

.balance-box .label {
    display: block;
    font-size: 0.85em;
    color: #6b7280;
}

.balance-box .amount {
    display: block;
    font-size: 1.25em;
    font-weight: 600;
    color: #1e40af;
}

.transactions {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.9em;
}

.transactions th,
.transactions td {
    padding: 0.5rem;
    border-bottom: 1px solid #e5e7eb;
}

.transactions th {
    background: #1f2937;
    color: white;
    text-align: left;
    font-weight: 500;
}

.transactions td:nth-child(3),
.transactions td:nth-child(4),
.transactions td:nth-child(5) {
    text-align: right;
    font-family: "Consolas", monospace;
}
''',
        },
    },
}


def cmd_init(args):
    """Initialize a new fullbleed project in the current directory."""
    target_dir = Path(args.path) if hasattr(args, "path") and args.path else Path.cwd()
    force = getattr(args, "force", False)
    
    # Check if already initialized
    config_path = target_dir / "fullbleed.toml"
    if config_path.exists() and not force:
        if getattr(args, "json", False):
            result = {
                "schema": "fullbleed.error.v1",
                "ok": False,
                "code": "ALREADY_INITIALIZED",
                "message": f"Directory already contains fullbleed.toml. Use --force to overwrite.",
            }
            sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        else:
            sys.stderr.write(f"[error] Directory already contains fullbleed.toml. Use --force to overwrite.\n")
        raise SystemExit(1)
    
    # Create directories
    created_dirs = []
    for dirname in DEFAULT_DIRS:
        dir_path = target_dir / dirname
        if not dir_path.exists():
            dir_path.mkdir(parents=True, exist_ok=True)
            created_dirs.append(dirname)
    
    # Create files
    created_files = []
    for filename, content in DEFAULT_INIT_FILES.items():
        file_path = target_dir / filename
        if not file_path.exists() or force:
            file_path.write_text(content, encoding="utf-8")
            created_files.append(filename)
    
    result = {
        "path": str(target_dir),
        "created_dirs": created_dirs,
        "created_files": created_files,
    }
    
    if getattr(args, "json", False):
        output = {"schema": "fullbleed.init.v1", "ok": True, **result}
        sys.stdout.write(json.dumps(output, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(f"[ok] Initialized fullbleed project in {target_dir}\n")
        if created_dirs:
            sys.stdout.write(f"  Created directories: {', '.join(created_dirs)}\n")
        if created_files:
            sys.stdout.write(f"  Created files: {', '.join(created_files)}\n")
        sys.stdout.write("\n  Next steps:\n")
        sys.stdout.write("    1. Edit fullbleed.toml or src/report.py\n")
        sys.stdout.write("    2. Install assets: fullbleed assets install inter\n")
        sys.stdout.write("    3. Build PDF:      fullbleed build\n")
        sys.stdout.write("    4. Watch mode:     fullbleed watch\n")


def cmd_new_template(args):
    """Create a new template from a starter template."""
    template_name = args.template
    target_dir = Path(args.path) if hasattr(args, "path") and args.path else Path.cwd()
    force = getattr(args, "force", False)
    
    if template_name not in TEMPLATES:
        available = ", ".join(TEMPLATES.keys())
        if getattr(args, "json", False):
            result = {
                "schema": "fullbleed.error.v1",
                "ok": False,
                "code": "UNKNOWN_TEMPLATE",
                "message": f"Unknown template: {template_name}. Available: {available}",
            }
            sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
        else:
            sys.stderr.write(f"[error] Unknown template: {template_name}\n")
            sys.stderr.write(f"  Available templates: {available}\n")
        raise SystemExit(1)
    
    template = TEMPLATES[template_name]
    created_files = []
    
    for filepath, content in template["files"].items():
        full_path = target_dir / filepath
        if full_path.exists() and not force:
            if getattr(args, "json", False):
                result = {
                    "schema": "fullbleed.error.v1",
                    "ok": False,
                    "code": "FILE_EXISTS",
                    "message": f"File already exists: {filepath}. Use --force to overwrite.",
                }
                sys.stdout.write(json.dumps(result, ensure_ascii=True) + "\n")
            else:
                sys.stderr.write(f"[error] File already exists: {filepath}. Use --force to overwrite.\n")
            raise SystemExit(1)
        
        full_path.parent.mkdir(parents=True, exist_ok=True)
        full_path.write_text(content, encoding="utf-8")
        created_files.append(filepath)
    
    result = {
        "template": template_name,
        "description": template["description"],
        "created_files": created_files,
    }
    
    if getattr(args, "json", False):
        output = {"schema": "fullbleed.new_template.v1", "ok": True, **result}
        sys.stdout.write(json.dumps(output, ensure_ascii=True) + "\n")
    else:
        sys.stdout.write(f"[ok] Created {template_name} template\n")
        for f in created_files:
            sys.stdout.write(f"  - {f}\n")
