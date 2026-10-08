# CodeQL review

The public repository runs CodeQL's extended security suites for JavaScript/TypeScript, Rust, and GitHub Actions. Review every finding; do not disable a query or add a broad source exclusion to make a scan pass.

The first scan identified missing gateway request limits. The gateway now uses `express-rate-limit` before sign-in/source handlers, export authentication, and static-file access. Its client-IP policy defaults to no proxy trust for direct traffic; production Compose trusts exactly one HTTPS proxy behind a loopback-only published port. Configure a fixed trusted chain and per-route limits for each deployment. A real-gateway regression verifies 429 responses, retry headers, resistance to spoofed forwarding headers, and access-token validation. Query and cookie export credentials share one verification step before redirecting or proxying.

Four initial findings require narrow, evidence-backed dismissals:

| Query | Location | Review |
| --- | --- | --- |
| `js/incomplete-multi-character-sanitization` | `parseUniversalDeck` style extraction | The regex extracts CSS for the player's style element. It does not establish an HTML safety boundary. Complete rendered slide HTML passes through DOMPurify in the browser; custom CSS is assigned using `textContent`. The browser regression includes malformed nested tags and event handlers. |
| `js/incomplete-multi-character-sanitization` | `extractSlideTitle` | The regex derives a display label. Labels use Vue's escaped text interpolation, and rendered slide HTML separately passes through DOMPurify. The label is not injected as raw HTML. |
| `rust/weak-sensitive-data-hashing` | `hash_token` | All stored session, participant, and OAuth tokens are generated from at least 32 cryptographically random bytes. SHA-256 supplies a deterministic lookup digest for those high-entropy bearer values. The app authenticates through Google and stores no human-chosen passwords. |
| `rust/weak-sensitive-data-hashing` | `oauth_browser_matches` | The comparison hashes two copies of a random 256-bit OAuth state and compares their fixed-size digests in constant time. The callback also requires a matching, unexpired server-side state and PKCE exchange. This is a random-state comparison rather than password derivation. |

Keep the original alerts and their dismissal explanations visible in GitHub's Security tab. Revisit these decisions if token generation, rendering, authentication, or proxy trust changes. New findings remain subject to review.
