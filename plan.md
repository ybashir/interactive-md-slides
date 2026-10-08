# Design and roadmap

Interdeck combines a private Rust/PostgreSQL API with a public Node/Vue gateway. Markdown is the canonical deck source. Stable slide and interaction IDs preserve response contracts across edits; presentation runs and result epochs keep rehearsals separate from durable results. PostgreSQL commits audience writes before acknowledging them, and LISTEN/NOTIFY fans state invalidations across replicas.

The browser player renders an approved Slidev subset without a worker per live deck. Explicit creator exports use a pinned upstream Slidev process in a temporary workspace, with restricted filesystem access and a minimal environment. Code-executing addons require a future isolation boundary and are currently disabled.

## Current release

Single-owner editing, autosave/recovery, five approved themes, image/font uploads, presenter preflight/navigation, room-code rotation, polls/quizzes/Q&A and the interaction catalog, CSV export, browser exports, optional Gemini assistance, and Docker self-hosting are implemented. Security checks cover OAuth browser binding, shared-result privacy, service secrets, exporter filesystem boundaries, and dependency patches. Data maintenance expires sessions, retries asset cleanup, and optionally prunes audience records.

## Next work

- Measure deployment-specific latency and failure recovery with simultaneous decks, gateway restarts, and PostgreSQL interruptions.
- Expand browser, screen-reader, mobile, and visual parity fixtures; reduce editor and chart startup payloads.
- Add immutable export artifacts, collaborative authoring, and finer creator access policies.
- Add an isolated execution boundary before admitting custom themes, addons, or server-side extensions.
- Improve assistant cancellation, usage budgets, and proposal review.

Operators must verify backups/restores and configure alerts for their own installation. Public release readiness does not establish a hosted-service SLA or a particular event capacity.
