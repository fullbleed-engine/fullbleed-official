<!-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial -->
# Fullbleed Licensing

Fullbleed is dual-licensed. You may use it under either:

1. `AGPL-3.0-only` (open source), or
2. `LicenseRef-Fullbleed-Commercial` (commercial license from Fullbleed).

If you are unsure which option applies to your use case, email `info@fullbleed.dev` and describe what you are building (CLI use vs Python bindings, redistributed product vs internal tooling vs SaaS).

This document is a practical guide for developers and procurement. It is not legal advice.

## 1) Option A: AGPL-3.0-only (Open Source)

Fullbleed is available under the GNU Affero General Public License v3 (`AGPL-3.0-only`).

You may use Fullbleed under AGPL at no cost, including commercial activity, as long as you comply with AGPL requirements.

In plain English, AGPL generally means (high level):

- You can run Fullbleed for any purpose.
- If you convey or distribute Fullbleed (or a derivative) to others, you must provide Corresponding Source under AGPL.
- If you modify Fullbleed and users interact with it over a network, you must offer those users Corresponding Source of the modified program (AGPL section 13).
- If your application is a derivative or combined work of Fullbleed (for example, by linking to Fullbleed as a library), distributing or hosting that combined work may require AGPL licensing for the combined work.

If you want to keep your product or service closed source, or you cannot (or do not want to) meet AGPL obligations, use Option B (Commercial).

SPDX identifier: `AGPL-3.0-only`

## 2) Option B: Fullbleed Commercial License (Revenue-Tiered)

Fullbleed offers a commercial license for organizations that want to use Fullbleed in proprietary or closed-source products and services, or otherwise avoid AGPL obligations.

### 2.1 What the commercial license is for

A commercial license is typically the right fit if you:

- Use Fullbleed (CLI or bindings) inside a closed-source product you distribute to customers.
- Use Fullbleed to provide a paid service (including "PDF generation as a service") and do not want AGPL copyleft obligations to apply to your broader solution.
- Need a procurement-friendly commercial grant (often including negotiated support, warranty language, and related terms).

If you can comply with AGPL and you are comfortable doing so, you do not need the commercial license.

### 2.2 Pricing (Annual Subscription)

Commercial licensing is priced by your organization's Annual Gross Revenue:

- `$240 / year` for `$0-$100,000`
- `$1,000 / year` for `$100,001-$1,000,000`
- `$5,000 / year` for `$1,000,001-$10,000,000`
- `$10,000 / year` for `$10,000,000 or more`

To obtain or renew a commercial license, contact:

- Email: `info@fullbleed.dev`
- Web: `fullbleed.dev`

### 2.3 Definitions (for pricing)

"Annual Gross Revenue" means your organization's gross revenue for the most recently completed fiscal year, including all revenue recognized by the legal entity using Fullbleed.

"Organization" means the legal entity using Fullbleed and its Affiliates.

"Affiliate" means any entity that controls, is controlled by, or is under common control with you, where "control" means ownership of more than 50% of voting securities or equivalent power to direct management.

If your procurement practice requires a different definition (for example consolidated GAAP revenue), contact us and we will align to your standard where possible.

### 2.4 How to buy or activate

Email `info@fullbleed.dev` with:

- Company or legal entity name
- Country or region
- Annual Gross Revenue tier
- Intended usage (CLI only vs Python bindings, internal vs distributed, SaaS vs on-prem product)
- Number of developers or deployments (optional)

We will respond with commercial license paperwork and billing details.

## 3) Quick "Which one do I need?" Guide

Generally OK under AGPL (no commercial license needed):

- Personal use, evaluation, hobby projects.
- Internal tooling used only within your organization (no external distribution), when you are comfortable complying with AGPL for modifications and applicable network interactions.
- Open-source products where you can license the combined work under AGPL-compatible terms.

Commercial license strongly recommended (or required if you will not comply with AGPL):

- A closed-source desktop or server product that bundles Fullbleed or embeds Fullbleed via Python bindings.
- A proprietary SaaS that uses Fullbleed to generate PDFs for customers where you do not want AGPL copyleft obligations to apply to your broader solution.
- Redistribution and white-label scenarios where customers receive Fullbleed directly or indirectly and you want clear rights without AGPL constraints.

## 4) Notes for Compliance and Procurement

- Fullbleed includes a `compliance` command to help teams generate license and compliance reports for third-party components.
- Fullbleed may emit a one-time licensing reminder in certain CLI flows. It is informational and does not change license terms.

## 5) Trademarks

"Fullbleed" and related marks or logos may be trademarks of Fullbleed. AGPL does not grant trademark rights. Do not use marks in ways that imply endorsement without permission.

## 6) Contact

For commercial licensing, procurement questions, or edge cases:

- Email: `info@fullbleed.dev`
- Web: `fullbleed.dev`
