# Security Policy

Zorya is early-stage Technical Preview software. Security reports are still handled as confidential product-security issues.

## Supported versions

Zorya follows a latest-preview support model until a stable release policy is defined.

| Version | Supported |
| --- | --- |
| Latest published Technical Preview | Yes |
| Older previews | No |

Security fixes normally land on `main` first and are included in the next published preview rather than being backported to older previews.

## Reporting a vulnerability

Use GitHub Private Vulnerability Reporting or the repository Security Advisory flow when available. Do not open a public issue, discussion or pull request for a vulnerability that could put users, credentials, release integrity or browsing data at risk.

If private GitHub reporting is unavailable, contact the repository owner privately through an existing GitHub profile contact method. Do not add new personal contact details to repository files solely for vulnerability reporting.

Include, when practical:

- affected revision or version;
- platform;
- reproduction steps;
- expected and observed security boundary;
- impact;
- the minimum proof-of-concept material required to understand the issue.

Do not include unrelated personal data, browsing history, credentials, tokens, private keys, signing material or other secrets.

## Triage process

- Initial acknowledgement target: within 3 business days.
- Initial severity and scope assessment target: within 7 business days.
- Critical and high-impact issues take priority over feature work.
- Valid issues are tracked privately until a fix or mitigation is available.
- Disclosure timing should allow affected users a reasonable opportunity to update.

These are response targets, not guarantees of a specific remediation date.

## Scope

Security-sensitive areas include:

- browser chrome spoofing or privilege confusion;
- navigation or origin display mismatches;
- stale tab/window/navigation work crossing identity boundaries;
- downloads and suggested filenames;
- external protocols and local-file handling;
- permission mediation;
- profile, history, bookmark and session data;
- secrets or credential storage;
- updater, installer, signing and release paths;
- CI/CD, dependency and artifact supply-chain integrity;
- Rarog host/content trust-boundary violations;
- native Windows integration.

Web-engine vulnerabilities in Rarog itself should be fixed in the Rarog repository, with the exact Zorya dependency revision updated afterward when applicable.

## Repository security model

The repository is expected to preserve:

- immutable full-SHA GitHub Action references;
- least-privilege workflow permissions;
- locked Cargo dependency resolution and exact Rarog Git revisions;
- CI, RustSec dependency audit, Gitleaks and CodeQL security gates;
- fail-closed release packaging and third-party license inventory checks;
- release artifact checksums and GitHub artifact attestations;
- immutable `v*` release tags once repository rules permit enforcement.

Never commit credentials, signing keys, private keys, tokens, service-account files, `.env` files, real browser profiles or generated secrets.
