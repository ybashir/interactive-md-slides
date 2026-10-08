# Interdeck

Interdeck is a self-hosted presentation app built on [Slidev](https://sli.dev). Write slides in Markdown, then add live polls, quizzes, word clouds, reactions, ratings, Q&A, and ranked voting. Audience members join from their phones with a room code; creators sign in with Google.

**Status: early release.** The app supports single-owner decks, autosave and recovery, private asset uploads, live presenter controls, anonymous or named participation, durable responses, and CSV downloads. Optional Gemini assistance helps with slide authoring and Q&A moderation. PDF, PPTX, and image exports use a creator-only Slidev worker. See [known limits](#known-limits) before choosing it for an event.

## Quick start

Requirements: Node **24.21.0** (tested; compatible ranges are 22.18+ on Node 22, or 24.11+), Rust **1.88+**, Docker Compose, and a Google OAuth web client. PostgreSQL 17 is provided by Compose.

```sh
git clone https://github.com/ybashir/interactive-md-slides.git
cd interactive-md-slides
npm ci
npm run setup
docker compose up -d postgres
```

`setup` creates a private `.env` with random service secrets and preserves an existing file. Edit it to set:

- `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` from your own Google OAuth **Web application** client.
- `GOOGLE_WORKSPACE_DOMAIN=example.org` to admit verified members of that Workspace, or `GOOGLE_ALLOWED_EMAILS=you@gmail.com,colleague@example.org` to admit specific verified Google accounts. You may combine the two. This policy controls **creators**, not audience members. An empty policy stops startup.
- `APP_BASE_URL=http://localhost:5173` for development.

In the Google client, add `http://localhost:5173/api/auth/google/callback` as an authorized redirect URI. Configure the consent screen and add your creator accounts as test users if the OAuth app is in testing. See [Google's web-server OAuth guide](https://developers.google.com/identity/protocols/oauth2/web-server).

```sh
npm run dev
```

Open <http://localhost:5173>. The API listens on loopback port 8080, the gateway on 3000, and Vite proxies both. Import a Markdown deck or open the bundled [Amazing Sea Creatures sample](samples/amazing-sea-creatures.md). The sample contains fictional presentation content and repository-owned artwork.

Google credentials belong to each installation. Forkers can configure their own domain or allowlist; nothing requires access to the original author's services. Gemini is disabled when its keys are empty, and ordinary presentations and manual Q&A work without it.

## Self-hosting

The public Node gateway serves the Vue app and proxies API/SSE requests. A private Rust API owns authentication, authorization, and PostgreSQL persistence. Slidev export workers run on loopback in temporary workspaces. The gateway receives only the two service secrets; Google, AI, database, and object-storage credentials belong to the API.

A single-host deployment is provided in [compose.production.yml](compose.production.yml):

1. Set `APP_BASE_URL=https://slides.example.org`, your Google credentials and access policy, and a random hexadecimal `POSTGRES_PASSWORD` in `.env`. Register `https://slides.example.org/api/auth/google/callback` with Google. Keep `INTERNAL_SERVICE_TOKEN` and `SLIDEV_TOKEN_SECRET` independent random values of at least 32 printable characters; `npm run setup` generates suitable values for new installs.
2. Put an HTTPS reverse proxy in front of `127.0.0.1:3000`. Preserve the public Host, forward WebSocket upgrades, and disable buffering/timeouts that terminate long-lived SSE. The Compose file publishes only the gateway to loopback; PostgreSQL and the API have no published ports.
3. Run `docker compose -f compose.production.yml up -d --build`. Use `docker compose -f compose.production.yml ps` to check health. Migrations run automatically on API startup. Back up before upgrading.

The Compose setup persists PostgreSQL and assets in named volumes and supports one API replica. For multiple API replicas, use a shared private S3-compatible bucket and set all `ASSET_S3_*` values from [.env.example](.env.example); leave `ASSET_STORAGE` empty. Local volume storage must be backed up with the database. Container-local files are not durable storage.

Choose an audience data policy before inviting users. `AUDIENCE_DATA_RETENTION_DAYS=0` retains responses until the owner deletes the deck or the operator removes them. A positive number prunes old participants, their responses/questions, and AI insight history on decks without a live run. Expired audience cookies lose access after one day; this does not delete saved results. Asset deletion is durable and asynchronous, with retry if storage is unavailable. Backups and infrastructure logs have independent retention policies. The app exposes the installation's AI/retention settings at `/privacy`; operators remain responsible for contact information, access requests, and provider terms.

Optional AI keys belong only on the API. `GEMINI_MODERATOR_KEY` enables Q&A analysis; `AI_MODERATION_MODE=assist` keeps decisions manual, while `enforce` may approve/reject pending questions with human overrides. `GEMINI_ASSISTANT_KEY` enables the authoring assistant. Q&A text and context, or assistant messages and deck content, are sent to Google when those features are used. The default model is `gemini-3.7-flash`; choose an available model from [Google's model catalog](https://ai.google.dev/gemini-api/docs/models) and review the provider's data policy for your account.

See [operations](ops/README.md) for backups, restore drills, monitoring, incident recovery, and upgrades.

## Interaction Markdown

A slide-level poll uses a stable ID and stable option IDs:

```md
:::interact{type="poll" id="roadmap-priority" results="after-vote"}
# What should we prioritize?

- [reliability] Reliability
- [delivery-speed] Delivery speed
:::
```

An individual bullet can be voteable:

```md
- Ship smaller changes more often {interact="updown" id="smaller-changes"}
- Celebrate this milestone {interact="reaction" id="milestone-reaction" options="like:👍|celebrate:🎉|question:🤔"}
```

Other initial contracts:

```md
:::interact{type="word-cloud" id="one-word-checkin"}
# Describe this quarter in one word
:::

:::interact{type="rating" id="confidence" min="1" max="5"}
# How confident are you?
:::

:::interact{type="poll" id="priorities" multiple="true" max="2" results="after-vote"}
# Choose up to two priorities
- [quality] Quality
- [speed] Speed
- [learning] Learning
:::

:::interact{type="quiz" id="quick-check" correct="quality" results="manual" timer="30"}
# Which priority prevents rework?
- [quality] Quality
- [speed] Speed
:::

:::interact{type="reaction" id="room-reaction" results="always"}
# How does this land?
- [love] ❤️ Love it
- [thinking] 🤔 Thinking
- [question] ❓ Questions
:::

:::interact{type="number" id="delivery-estimate" min="0" max="100" step="1" unit="days"}
# Estimate the delivery time
:::

:::interact{type="allocation" id="investment" total="100"}
# Allocate 100 points
- [quality] Quality
- [speed] Speed
- [learning] Learning
:::

:::interact{type="matrix" id="priority" x-min="0" x-max="10" x-label="Effort" y-min="0" y-max="10" y-label="Impact"}
# Place this initiative
:::

:::interact{type="image-choice" id="direction"}
# Choose a visual direction
- [calm] Calm {image="https://example.com/calm.png"}
- [bold] Bold {image="/api/decks/DECK_ID/assets/ASSET_ID/content"}
:::

:::interact{type="ranking" id="priority-order" results="after-vote"}
# Put every priority in order
- [quality] Quality
- [speed] Speed
- [learning] Learning
:::

:::interact{type="image-hotspot" id="focus-map" image="https://example.com/map.png" alt="Office floor plan" results="after-vote"}
# Where should the collaboration space go?
:::

:::interact{type="survey" id="team-pulse" results="presenter"}
# Quick pulse
- [confidence] How confident are you? {type="rating" min="1" max="5"}
- [priority] Which priority matters? {type="choice" options="quality:Quality|speed:Speed|learning:Learning"}
- [comment] What should improve? {type="text" max="500" required="false"}
:::

::audience-qr{size="180"}
```

`results="manual"` keeps aggregate results hidden on audience devices and the projected slide until the presenter reveals them. `results="on-close"` reveals them when input closes. A `timer` between 5 seconds and two hours adds synchronized start, pause/resume, and reset controls to the presenter console.

Image-choice options and hotspot backgrounds accept HTTPS images or private uploaded deck-asset URLs. Uploaded images remain owner-only outside a presentation; joined participants receive a live-room-scoped path while that run is active. Hotspots require descriptive `alt` text and include keyboard-adjustable coordinate controls rather than relying on pointer input alone.

A survey is one atomic interaction and may contain 1–10 stable-ID rating, choice, or text questions. Choice options use stable `id:Label` pairs separated by `|`; text questions can be optional. The editor inserts a safe presenter-only-results default, which avoids exposing free-text answers unless the creator deliberately chooses another result policy.

An interaction can also be the entire slide. Ranked-list voting opens automatically: audience members vote on each option as it appears through ordinary Slidev clicks. The order remains stable until the presenter uses the fullscreen **Close voting** control, which animates the list into score order:

```md
:::interact{type="ranked-list" id="priority-rank" display="slide" reveal="click" results="on-close"}
# Which ideas matter most?

- [quality] Quality
- [speed] Speed
- [learning] Learning
:::
```

Every slide is assigned a stable marker such as `<!-- interdeck-slide: priorities -->`. Interdeck inserts missing markers on save. Duplicate or malformed interaction IDs reject the revision instead of silently corrupting historical results.

## Slidev compatibility

Interdeck pins Slidev's parser and exporter. Normal preview and presentation run inside one prebuilt browser player, so a deck does not own a Vite process. The first universal-player slice supports standard Markdown and safe HTML, deck and slide frontmatter, the common default/center/cover/intro/fact/statement/section/two-column layouts, fade and directional transitions, `v-click`/`v-clicks`, private assets, deck-wide and slide-local CSS, and every Interdeck interaction. The editor theme selector retains Default, Seriph, Apple Basic, Bricks, and Shibainu; visual-parity fixtures for the more specialized theme layouts remain hardening work.

The standard creator export deliberately uses the pinned upstream Slidev runtime, preserving its PDF/PPTX/image workflow. Advanced executable Vue components, arbitrary addons, Mermaid/PlantUML/LaTeX compilation, drawings, and the full UnoCSS surface are not silently executed inside the shared player; each will be added as a reviewed prebundled capability or routed through a future isolated compatibility path.

Themes and addons are executable packages. Community packages are therefore not installed from deck Markdown at runtime. An unapproved theme or addon produces an actionable compatibility message in the preview; adding one requires review, an exact dependency pin, and a compatibility fixture.

`npm run check:slidev-compatibility` starts an actual transformed Interdeck deck under Default, Seriph, Apple Basic, Bricks, and Shibainu to protect the export path. The GitHub Actions workflow runs this fixture alongside formatting, warning-free Clippy, application tests, production builds, script syntax checks, and dependency auditing at every severity.

Preview and Present load the same cached player bundle and fetch only deck data, so simultaneous authors no longer consume native worker slots. The gateway's existing bounded worker manager remains only for explicit exports and as a temporary rollback boundary while universal-player parity is hardened.

## Development and verification

```sh
npm test
npm run check
npm run check:slidev-compatibility
npm run smoke:slidev-lifecycle
npm run smoke:export-isolation
npm run audit:dependencies
npm run audit:rust # requires cargo-audit 0.22.2
# With the disposable local API/gateway running:
npx playwright install chromium
npm run test:browser
cargo fmt --manifest-path services/app/Cargo.toml --check
cargo clippy --manifest-path services/app/Cargo.toml --all-targets -- -D warnings
```

Database regressions need a disposable local PostgreSQL database and running API. Set `DATABASE_URL` explicitly, plus `SMOKE_BASE_URL` for `npm run smoke:security` and `SMOKE_API_BASE` for `npm run smoke:interaction-locks`. Run two APIs against that database for `npm run smoke:cross-replica -- --writer-url=http://127.0.0.1:8080 --listener-url=http://127.0.0.1:8081`. For the migration and durable-cleanup test, set `INTERDECK_TEST_DATABASE_URL` and run `cargo test --manifest-path services/app/Cargo.toml maintenance::tests::database_cleanup -- --ignored`. The scripts refuse remote targets by default and clean up their synthetic fixtures. CI runs these checks with dummy credentials and an isolated service database.

`npm run load:seed` and `npm run load:audience` provide a local 1,000-client SSE/vote scenario. This is a benchmarking tool, not a capacity guarantee. Measure your own isolated deployment, including reconnect storms, provider outages, and simultaneous decks, before a large event.

Dependency auditing includes development dependencies. Two upstream packages currently need exact-source, hash-verified local security patches applied by `npm ci`; see [dependency security](docs/dependency-security.md). The audit gate accepts only those verified advisory IDs and rejects new vulnerabilities. Raw `npm audit` still reports the upstream packages until their maintainers release fixes.

## Known limits

- This release is self-hosted software with no hosted-service SLA or universal audience-capacity claim. Backup scheduling, alerts, HTTPS, and provider accounts are operated by the installer.
- Each deck has one owner. Collaborative authoring, arbitrary npm addons/community themes, and server-generated immutable export artifacts are future work.
- Five pinned themes are supported: Default, Seriph, Apple Basic, Bricks, and Shibainu. Uploaded decks cannot import local files or use `<<<` snippets; paste the source or upload an asset. Export uses a restricted filesystem and removes service credentials from the worker environment. Authors should be trusted: the renderer is not a container sandbox for arbitrary plugins or Node code.
- The universal player implements a reviewed Slidev subset. Complex custom Vue components, executable plugins, or unusual layouts may require adaptation. Export fidelity can differ from the live player; check your deck before presenting.
- Uploaded assets are signature-validated raster images, static sanitized SVG, and WOFF/WOFF2 fonts. External images and theme fonts may contact third-party servers. Use uploaded assets/fonts for private or offline presentations.
- Basic keyboard controls and readable typography are covered; full assistive-technology certification and visual parity across every browser are ongoing. See the [roadmap](plan.md).

## Contributing and security

Read [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for community expectations. Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).

Keep credentials, real customer decks, participant data, and infrastructure logs outside Git. `.env*`, `.private/`, and generated artifacts are ignored by Git and Docker. Contributed samples must be fictional or public and include redistribution rights for every bundled asset.

## License

Interdeck is [MIT licensed](LICENSE), copyright 2026 Yasser Bashir. The bundled original [sample artwork](apps/web/public/samples/amazing-sea-creatures/README.md) is also MIT licensed. Dependencies retain their own licenses and notices, including Slidev; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Browser builds and API containers also generate notices for their compiled dependencies.
