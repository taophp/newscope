# MyNewsLens — Roadmap

This roadmap outlines the planned development path for MyNewsLens. It breaks the project into prioritized phases with clear milestones, acceptance criteria, and suggested checkpoints. The aim is to deliver a useful, secure, and maintainable product that runs well on Raspberry Pi while remaining extensible for more powerful deployments.

---

## Vision recap (1 line)
Help an individual spend limited, planned time on high-quality news monitoring and research by automatically aggregating, prioritizing and summarizing relevant items, and by enabling a focused conversation with an AI assistant.

---

## Roadmap Overview (high-level phases)

- Phase 0 — Project scaffolding & design
- Phase 1 — MVP: Core ingestion, scheduling, summary & chat
- Phase 2 — Improved AI behaviors & scoring
- Phase 3 — Advanced extraction, search & UX improvements
- Phase 4 — Production hardening, packaging & community
- Ongoing — Maintenance, monitoring, community contributions

Each phase lists milestones, acceptance criteria, and optional stretch goals.

---

## Phase 0 — Project scaffolding & design (Foundations)
Goal: Create a developer-ready repository skeleton, basic configurations and initial CI so the team can iterate safely.

Milestones
- Create Cargo workspace: `server`, `worker`, `common`, optional `cli`.
- Add `README.md`, `ROADMAP.md`, `CONTRIBUTING.md`, `CODING_GUIDELINES.md`.
- Add `docker-compose.yml` skeleton for multi-service local development.
- Initialize `sqlx` migrations folder and an initial schema migration.
- Add `config.example.toml` describing configuration options.
- Basic GitHub Actions pipeline for linting (rustfmt, clippy) and tests.

Acceptance criteria
- Repo builds (`cargo build --all`) on x86_64 and cross-build instructions documented for RPI.
- `docker-compose up` launches services with a stub server and worker (no external LLM).
- `sqlx migrate` can create the initial SQLite DB.

Estimated effort
- 1–2 sprint days (solo developer), depending on CI familiarity.

Stretch goals
- Create issue templates and PR templates for GitHub.
- Add initial architecture diagram under `docs/`.

---

## Phase 1 — MVP (Minimum Viable Product)
Goal: Provide the core user experience: scheduled feed ingestion, summarization, basic chat, and session timers.

Milestones
- Implement worker scheduler using `tokio-cron-scheduler` with default times (05:00, 11:00, 17:00, 23:00).
- Feed ingestion pipeline using `feed-rs`.
  - Store feed metadata and items in SQLite via `sqlx`.
  - For short items, fetch linked page and extract content via `scraper`.
- Implement a lightweight "page-to-RSS" heuristics module to produce simple feeds when none exist (list extraction heuristics).
- Basic scoring pipeline:
  - Redundancy count (how many feeds report the item)
  - Recency
  - Explicit user preferences (keyword matching)
- Provide a simple LLM abstraction (trait) and a remote connector placeholder (can be disabled).
  - Use remote LLM for summary generation in MVP (mock mode allowed).
- Rocket-based web server:
  - Auth via local config file (single-user flow).
  - REST endpoints for feed management, OPML import, session start/stop.
  - WebSocket or SSE chat endpoint.
- Basic web UI (vanilla JS):
  - Import OPML or add feeds
  - Start a timed session, view the generated summary (half of requested reading time), and chat with the assistant.
- Persist sessions and chat messages into DB.

Acceptance criteria
- Worker runs on schedule and ingests feeds successfully from a test OPML file.
- Starting a session produces a generated summary of aggregated items since the last session and displays it in the UI.
- Timer in the UI is informational and triggers a visual notification when elapsed.
- Chat messages are stored and associated with sessions.
- The application runs on an RPI3 in a degraded mode (remote LLM disabled or very small).

Estimated effort
- 2–4 sprints, depending on UI polishing and integration with an LLM connector.

Stretch goals
- Add a simple local LLM connector stub that returns extractive summaries until a real model is integrated.
- Allow importing feeds in parallel (with concurrency limits).

---

## Phase 2 — Improved AI behaviors & scoring
Goal: Enhance the relevance of results by learning from conversations and using semantic de-duplication.

Milestones
- Implement feedback capture in conversation:
  - Implicit signals: messages that indicate interest or disinterest.
  - Explicit signals: thumbs-up/down on items or summaries.
- Preference learning:
  - Represent user preferences in a structured profile (topics, blocked categories, preferred sources).
  - Update preferences from chat analysis and explicit feedback.
- Semantic de-duplication:
  - Integrate LLM-based semantic comparison to detect cross-posted or near-duplicate stories.
  - Use a hybrid approach: cheap hash/levenshtein pass + semantic LLM pass for borderline cases.
- Serendipity & frequency-aware scoring:
  - Give small boost for low-frequency sources (blogs with rare but potentially high-value posts).
  - Add tunable serendipity parameter in scoring formula.
- Local LLM option:
  - Add support for local inference via a lightweight engine (pluggable, e.g., ggml/llama.cpp bindings).
  - Provide clear config and fallbacks to remote LLM if local is unavailable.
- Logging of LLM usage (tokens or inference time) and graceful failure when quotas exceed or model is missing.

Acceptance criteria
- The system updates user preference state from conversational signals and reduces appearance of unwanted categories in subsequent summaries.
- Semantic de-duplication significantly reduces duplicate stories across feeds in test cases.
- A local LLM can be started and returns usable summaries on a small dataset (even if lower quality than remote).

Estimated effort
- 3–6 sprints (LLM integration and robust feedback loop are intensive).

Stretch goals
- Add embeddings caching and inexpensive vector similarity for faster semantic checks (can be a simple in-memory or file-backed cache).

---

## Phase 3 — Advanced extraction, search & UX improvements
Goal: Improve content extraction quality, add powerful search, and refine the user experience.

Milestones
- Advanced "page-to-RSS" extraction:
  - Provide a rule-based extraction engine with fallbacks (content block detection, link list extraction).
  - Allow users to create and persist extraction templates for sites.
- Full-text index & search using `tantivy`:
  - Index articles, summaries, and chat transcripts.
  - Provide search endpoint and UI.
- Improved UX:
  - Persist summary panel visible alongside chat (split layout when resolution allows).
  - Allow users to rename and tag sessions, mark favorites.
  - Provide a small dashboard with feed health, ingestion stats, and preference summary.
- Internationalization:
  - Add English and French localization for UI and system messages.
  - Prepare the codebase to accept more languages later.
- Respectful scraping policies and politeness:
  - Per-domain concurrency and delay configuration.
  - Optional robots.txt honoring toggle in config (default off for a personal app but documented).

Acceptance criteria
- Extraction templates improve yield on tested sites that lack RSS.
- Search returns relevant results and supports queries across sessions and articles.
- The split-panel UI remains responsive on typical desktop and tablet viewports.

Estimated effort
- 4–8 sprints.

Stretch goals
- Provide a simple visual extractor UI for users to click/select article lists on a page to create a template.

---

## Phase 4 — Production hardening, packaging & community
Goal: Make the project easy to deploy, manage and contribute to; prepare releases and developer documentation.

Milestones
- Production Docker images and `docker-compose` files for x86_64 and ARM (multi-arch builds via buildx).
- Hardened configuration and secrets handling:
  - Document environment variables and secure storage recommendations.
- Add robust testing:
  - Integration tests for worker + db + server.
  - End-to-end tests for a basic ingestion-to-summary flow (mock external HTTP).
- Monitoring and metrics:
  - Basic metrics endpoints (ingestion counts, last run times, LLM failures).
  - Structured logging and recommended log rotation for long-running installs.
- Documentation: `docs/DEPLOYMENT.md`, `docs/UPGRADE.md`, and `docs/CONTRIBUTING.md`.
- Release (v0.1.0) with packaging instructions and example RPI setup guide.

Acceptance criteria
- Automated CI builds a release artifact and multi-arch images on tags.
- A documented deployment successfully runs on a clean RPI3 with the provided instructions.

Estimated effort
- 3–6 sprints.

Stretch goals
- Optionally provide a small installer script for RPI (non-invasive, documented).

---

## Ongoing — Maintenance, community & features backlog
These items are evergreen and evolve from community feedback and usage patterns.

- Collect user feedback and prioritize feature requests.
- Implement notifications (desktop, mobile) as optional features.
- Add multi-user management and per-user quotas (if the project evolves toward a hosted offering).
- Add plugin/connectors for third-party services (e.g., Pocket, Readwise) as integrations.
- Performance optimizations for large feed sets (throttling, batching, incremental indexing).

---

## Suggested milestones & timeline (example)
Note: timelines are contextual and depend on available developer time and contributors.

- Month 0, Weeks 1–2: Phase 0 complete.
- Month 1, Weeks 3–6: Phase 1 MVP alpha.
- Month 2–3: Phase 2 core AI improvements and local LLM option.
- Month 4: Phase 3 advanced features and search.
- Month 5: Phase 4 release candidate and documentation.

---

## Priorities & trade-offs
- Keep RPI constraints in mind: memory and CPU are limited. Local LLMs must be optional and lightweight.
- Favor simplicity in the early phases. It’s better to ship a stable core than many half-built features.
- Make the LLM integration pluggable: users should be able to choose local or remote models without code changes.

---

## How to contribute to the roadmap
- Open issues for proposed features and label them according to priority (P0, P1, P2).
- Submit PRs that target a single milestone and include tests for the behavior they add.
- Discuss large design changes (e.g., switching front-end stack to Yew) in an issue before implementing.

---

## Closing notes
This roadmap is a living document. It should be revisited at every release or after major community feedback. The goal is to stay pragmatic: deliver a lightweight, self-hostable assistant that helps real people spend less time and gain more insight from their news sources.
