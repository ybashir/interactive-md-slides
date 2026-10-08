# Security policy

Security fixes target the latest release and `main`. Older revisions are not separately maintained. This is early self-hosted software; installers must configure HTTPS, private API/database networking, creator access, backups, and provider credentials.

Use the repository's **Security → Advisories → Report a vulnerability** flow to report suspected vulnerabilities privately. If that button is unavailable, open an issue asking for a private contact method without disclosing exploit details, secrets, or affected user data. GitHub documents the [private reporting process](https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/report-privately).

Include the affected revision, a minimal reproduction against your own disposable installation, impact, and a suggested fix if available. Do not test someone else's hosted installation without its operator's permission. We will coordinate a fix and disclosure, but do not promise a specific response time.

Keep service credentials on the intended service only. Rotate any exposed key; removing it from Git does not revoke it. Audience room codes allow joining an active room and should be shared deliberately. Shared SSE never includes protected after-vote/manual/presenter-only results; authorized participant results use the session-bound REST endpoint.

Export workers are restricted to their own workspace and installed dependencies, with no API/Google/database secrets in the child environment. Arbitrary file imports and addons are disabled. This is not a general-purpose sandbox for untrusted executable extensions; installations should admit trusted deck creators.

The app publishes its retention and AI configuration at `/privacy`. Deployment operators handle privacy requests and backup/log retention. Read [dependency security](docs/dependency-security.md) for the narrowly scoped upstream advisory exceptions and their verified patches.
