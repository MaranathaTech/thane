# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in thane, please report it responsibly. **Do not open a public GitHub issue.**

Report via our **[contact form](https://getthane.com/contact)** with "Security" as the subject.

Include:

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if you have one)

We will acknowledge your report within 48 hours and aim to provide a fix or mitigation within 7 days for critical issues.

## Scope

The following are in scope:

- Sandbox escapes (Landlock, seccomp, App Sandbox bypasses)
- Audit log tampering or bypass
- RPC/IPC authentication or authorization issues
- Sensitive data leaks (credentials, PII) through logs or IPC
- Arbitrary code execution via crafted input

The following are out of scope:

- Issues that require physical access to the machine
- Denial of service against the local application
- Social engineering attacks
- Issues in dependencies (report those upstream, but let us know)

## Supported Versions

We provide security fixes for the latest release only.

| Version | Supported |
|---------|-----------|
| Latest  | Yes       |
| Older   | No        |

## Acknowledgments

We appreciate responsible disclosure and will credit reporters in release notes (unless you prefer to remain anonymous).
