# Security Policy

Zorya is early-stage software and is not yet production-ready, but security issues should still be handled privately.

## Reporting a vulnerability

Do not open a public issue for a vulnerability that could put users or their data at risk.

If GitHub private vulnerability reporting or Security Advisories are available for this repository, use that channel. Otherwise contact the repository owner privately through an available GitHub profile contact method.

Include, when practical:

- affected revision or version;
- platform;
- reproduction steps;
- expected and observed security boundary;
- impact;
- any proof-of-concept material needed to understand the issue.

Do not include unrelated personal or browsing data.

## Scope

Security-sensitive areas include:

- browser chrome spoofing or privilege confusion;
- navigation or origin display mismatches;
- downloads and suggested filenames;
- external protocols and local-file handling;
- permission mediation;
- profile, history, bookmark and session data;
- secrets or credential storage;
- updater, installer and signing paths;
- Rarog host/content trust-boundary violations;
- native Windows integration.

Web-engine vulnerabilities in Rarog itself should be fixed in the Rarog repository, with the Zorya dependency updated afterward when applicable.
