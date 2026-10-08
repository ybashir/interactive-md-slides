# Dependency security

The lockfiles and Slidev 52.19.1 catalog are committed. `npm ci` applies two temporary local patches through `scripts/patch-dependencies.mjs`. The patch manifest includes the exact version, original source SHA-256, modified source SHA-256, advisory ID, and replacement. An unexpected version or source hash fails installation; review a dependency update before changing the manifest.

| Package | Advisory | Mitigation |
| --- | --- | --- |
| braces 3.0.3 | GHSA-vfj7-8cjw-p6xm | Bound nested parser depth before recursive compilation/stringification. |
| sprintf-js 1.0.3 | GHSA-hp3w-g68c-fv3c | Reject width or precision above 10,000 before string allocation. |

`npm run test:security-patches` verifies the installed hashes, ordinary behavior, and malicious inputs. `npm run audit:dependencies` audits all dependency classes and accepts only the two advisory IDs when the exact single installed copy is patched. It fails on new advisories, nested unpatched copies, missing audit data, or changed source. Raw `npm audit` still flags the published upstream versions and their dependency chains. Remove each exception when a verified upstream fix is available.

Rust uses patched rustls 0.23.45. Cargo.lock also records rsa through SQLx's optional MySQL driver. The PostgreSQL-only build does not compile it. `npm run audit:rust` proves rsa is absent from the active normal/build graph before excluding **RUSTSEC-2023-0071** from `cargo audit`; enabling MySQL or another active rsa path fails the gate and requires a fix. No other vulnerability advisory is ignored.

CI runs scheduled and pull-request audits. Dependabot proposes npm, Cargo, and action updates; action versions are pinned to full commit IDs. CodeQL runs for public repositories, and Gitleaks scans the complete reachable Git history. A passing audit establishes the checked advisory state and patches, not an absence of unknown vulnerabilities.
