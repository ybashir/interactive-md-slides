# Contributing

Interdeck welcomes bug reports, documentation improvements, and focused pull requests. Use the setup in [README.md](README.md), a local disposable database, and fictional deck/audience data. Never commit `.env`, real presentation content, credentials, or production logs.

Before a pull request, run `npm test`, `npm run check`, Rust formatting and Clippy, and the checks relevant to your change. Authentication, result visibility, storage, or presentation changes need database regressions; Slidev changes need compatibility and export-isolation checks. CI runs the complete release checks. Add tests for meaningful behavior or regressions, rather than duplicating implementation details.

Keep changes focused and explain the problem, resulting behavior, and validation in the PR description. Preserve stable interaction IDs and migrations; do not edit an already-released migration. Dependency changes must preserve the exact Slidev compatibility catalog and pass the audited local-patch checks. Review all new assets and dependency licenses.

Report security issues through [SECURITY.md](SECURITY.md), not a public issue. For ordinary bugs, include the version, browser/runtime, reproducible steps, expected/actual behavior, and a sanitized minimal deck. No response-time guarantee is made.

Contributions are submitted under the project's MIT license. Contribute only code and assets you have the right to redistribute; retain upstream copyright and license notices. No contributor license agreement is required.
