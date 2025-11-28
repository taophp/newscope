# MyNewsLens — Product Specification (Cahier des Charges)

Version: 0.1
Date: 2025-11-13
Authors: MyNewsLens core team

Table of contents
- 1. Purpose & Problem Statement
- 2. Goals & Success Metrics
- 3. Target Users & Use Cases
- 4. Scope (MVP vs Future)
- 5. Functional Requirements
  - 5.1 User & Configuration
  - 5.2 Feed Management & Import
  - 5.3 Ingestion & Scraping
  - 5.4 Deduplication & Aggregation
  - 5.5 Scoring & Prioritization
  - 5.6 Deep-dive / Content enrichment
  - 5.7 Summarization & LLM Integration
  - 5.8 Session / Chat UX
  - 5.9 Feedback & Preference Learning
  - 5.10 Persistence & History
  - 5.11 Operational controls (politeness, robots.txt)
- 6. Non-Functional Requirements
  - 6.1 Performance & Resource Constraints
  - 6.2 Reliability & Availability
  - 6.3 Security & Privacy
  - 6.4 Portability & Deployment
  - 6.5 Observability & Debuggability
  - 6.6 Maintainability & Extensibility
- 7. Data Model (high level)
- 8. APIs & Interfaces (high level)
- 9. Scheduler & Runtime Behavior
- 10. LLM Design & Requirements
- 11. Scraping / "Page → RSS" conversion heuristics
- 12. UX / UI Requirements
- 13. Configuration & Admin
- 14. Testing & Acceptance Criteria
- 15. Roadmap (short bullets of next steps)
- Appendix A: Constraints & Decisions taken for MVP

---

1. Purpose & Problem Statement
------------------------------
MyNewsLens helps a single user (initial MVP) perform focused, time-boxed news monitoring and research across a set of sources (RSS feeds, imported OPML, or websites without RSS). Instead of aimless scrolling, the system aggregates, prioritizes, and summarizes new content and lets the user interact with an assistant (LLM) to deepen comprehension — all within a planned time budget.

Problems solved:
- Reducing time wasted following many sources manually.
- Surfacing the most important, novel, and relevant items spanning multiple feeds.
- Turning disparate signals (feed headlines, short items, web pages) into a single, ranked, readable digest.
- Allowing interactive follow-up and preference capture through a conversational flow.

2. Goals & Success Metrics
--------------------------
Primary goals (MVP):
- Produce a concise, prioritized summary of new items since last session.
- Provide a conversational UI where the user can explore and refine relevance.
- Run on a Raspberry Pi 3+ (resource-constrained) by default, while allowing remote LLM fallback.

Success metrics:
- Time to produce digest after scheduled ingestion: < 2 minutes for datasets typical of a single user (<= 500 items/hour).
- Median size of digest such that user reading time ≈ half of chosen session time.
- User-reported relevance (post-session rating) >= 75% for first 100 sessions (goal).
- System runs on RPi 3+ within memory/CPU limits for regular ingestion (no heavy local LLM usage by default).

3. Target Users & Use Cases
---------------------------
Primary users:
- Knowledge workers, researchers, developers, product managers who want planned daily/periodic news/tech monitoring.
- Single-user installs (initially local) with potential future multi-user deployments.

Primary use cases:
- Daily 20-minute technology news check: the user requests "20 minutes", receives a summary fitting 10 minutes, and then uses the remaining 10 for conversation with the assistant.
- Import a list of feeds from another reader (OPML) and start receiving consolidated digests.
- Add a website that has no feed; MyNewsLens converts it heuristically into a feed so the site can be monitored.

4. Scope (MVP vs Future)
------------------------
MVP (deliverable):
- Single-user configuration via file.
- Feed ingestion (RSS/Atom) via `feed-rs`.
- Site scraping for short items and heuristics to generate feed-like items.
- Hourly ingestion scheduler using configured times (default: 05:00, 11:00, 17:00, 23:00 local time).
- Deduplication using hashing plus LLM-assisted semantic deduplication when LLM is available.
- Priority scoring combining redundancy, recency, source weight, user explicit preferences.
- Summarization via an LLM abstraction (supports local models if available, or remote models).
- Web UI (lightweight JS) with chat, summary panel, and informational timer.
- SQLite storage using `sqlx`.
- Docker Compose multi-arch packaging for deployment.

Out of scope for MVP (roadmap items):
- Full-text index + search (`tantivy`).
- Push notifications (mobile).
- Full multi-user account management & quotas.
- Advanced model selection UI & quota management.
- Advanced "page→RSS" ML models beyond heuristic rules.

5. Functional Requirements
--------------------------

5.1 User & Configuration
- FR-UC-01: Support an initial single-user created through a configuration file.
- FR-UC-02: Allow the user to update their display name and preferred language (fr/en).
- FR-UC-03: Expose a configuration file (`config.toml`) for scheduler times, politeness params, robots.txt behavior, and LLM connection details.

5.2 Feed Management & Import
- FR-FEED-01: Allow adding/removing individual feed URLs via UI or config.
- FR-FEED-02: Support import of OPML files; import should add all valid feed URLs in parallel (bounded concurrency).
- FR-FEED-03: Keep per-feed metadata: title, site URL, last_checked, status, user-assigned weight (optional).
- FR-FEED-04: No hard-coded limit on number of feeds in DB; ingestion concurrency controlled by config.

5.3 Ingestion & Scraping
- FR-ING-01: Periodic ingestion at configured times. Default schedule: 05:00, 11:00, 17:00, 23:00 local time.
- FR-ING-02: Fetch feeds using `reqwest` asynchronously and parse with `feed-rs`.
- FR-ING-03: For feed items that contain minimal content, fetch the linked page and extract the main article content using `scraper`.
- FR-ING-04: For sites without feeds, provide a "site monitor" mode: given a site URL, attempt to discover lists of items and normalize into a feed. This uses heuristics (see section 11).
- FR-ING-05: Respect politeness (per-domain concurrency & delay) by default. Respect robots.txt is optional (configurable).
- FR-ING-06: Enforce a maximum download size (default 512 KB) and a timeout for network requests (default 10s).
- FR-ING-07: Limit scraping depth: by default do not follow links beyond the immediate link if the item content is below a short-content threshold (e.g., < 100 characters).

5.4 Deduplication & Aggregation
- FR-DEDUP-01: Track article occurrences across feeds to compute redundancy counts.
- FR-DEDUP-02: Compute canonical URL and content hash for deduplication.
- FR-DEDUP-03: When available, run semantic deduplication and clustering via LLM embeddings or LLM-assisted similarity checks.
- FR-DEDUP-04: Maintain an occurrence table that maps canonicalized articles to feed items and first seen timestamps.

5.5 Scoring & Prioritization
- FR-SCORE-01: Produce a score per article combining:
  - redundancy_count (higher means more important),
  - recency (newer more weight),
  - source weight (user-tunable),
  - explicit user preferences match (positive boost),
  - novelty penalty (if very similar to previously seen content).
- FR-SCORE-02: Provide a configurable weighting scheme in config for the scoring components.
- FR-SCORE-03: Include a small "serendipity boost" factor to occasionally surface low-frequency sources.

5.6 Deep-dive / Content enrichment
- FR-DEEP-01: If the article content is short (<100 chars) and the article score passes a threshold, fetch the linked page and extract a fuller content (depth=1).
- FR-DEEP-02: Extract metadata (title, author, publish date) and the main textual body.
- FR-DEEP-03: Do not execute or store JavaScript; sanitize HTML before any rendering.

5.7 Summarization & LLM Integration
- FR-LLM-01: Provide an LLM adapter interface that can be implemented by:
  - local quantized models (e.g., llama.cpp bindings),
  - remote providers via HTTP API.
- FR-LLM-02: Summaries must be hierarchical: short headline summary, 3-7 bullet points, and optional expandable details.
- FR-LLM-03: The digest length should be configurable to target a reading time equal to half of the user’s requested session time.
- FR-LLM-04: LLM calls must be timeout protected and handle errors gracefully (fallback to extractive summary if LLM fails).
- FR-LLM-05: Use the LLM for semantic deduplication, relevance re-ranking, and generating user-facing concise summaries.
- FR-LLM-06: Track usage metadata (tokens or local compute time) to allow reporting/errors (no quota enforcement in MVP).

5.8 Session / Chat UX
- FR-CHAT-01: When the user starts a session, present the summary and start an informational timer (client-side).
- FR-CHAT-02: Allow the user to read the summary while conversing; summary panel should be viewable alongside the chat if screen size allows.
- FR-CHAT-03: Conversation persists as a session object with timestamped messages and linkable to the digest that triggered it.
- FR-CHAT-04: Chat messages are stored with speaker (user/assistant), text, and timestamp.
- FR-CHAT-05: The system must interpret user feedback inline (e.g., "I don't want sports") and update preference model.

5.9 Feedback & Preference Learning
- FR-PREF-01: Capture explicit feedback controls in digest UI (e.g., upvote/downvote per item).
- FR-PREF-02: Capture implicit feedback from conversation: mentions of topics, categories, or explicit exclusions.
- FR-PREF-03: Persist a simple preference model (keywords, categories, source weights) and use it in scoring.
- FR-PREF-04: Provide a facility to export/import preferences (JSON).

5.10 Persistence & History
- FR-HIST-01: Store all session digests and chat history in SQLite.
- FR-HIST-02: Allow the user to list and reopen past sessions with the full transcript and digest context.
- FR-HIST-03: Data retention is configurable; default preserves history indefinitely.

5.11 Operational controls (politeness, robots.txt)
- FR-OPS-01: Respect per-domain concurrency and delay parameters (default delay: 1s; default concurrency: 2).
- FR-OPS-02: Respect robots.txt behavior toggle (default: false; user chooses).
- FR-OPS-03: Provide operational logs and a way to inspect last ingestion run, per-feed errors.

6. Non-Functional Requirements
------------------------------

6.1 Performance & Resource Constraints
- NFR-PERF-01: The system must run on Raspberry Pi 3+ for basic functionality (ingestion and scheduling). Local LLM usage on RPi is optional and must degrade gracefully.
- NFR-PERF-02: Default ingestion schedule should be lightweight (bounded concurrency) to avoid CPU/IO saturation.
- NFR-PERF-03: Maximum memory footprint for a standard run (without heavy local LLM) should be below available RPi RAM (<1.5 GB in practice).

6.2 Reliability & Availability
- NFR-REL-01: The worker scheduler must be robust to transient network failures (retries + backoff).
- NFR-REL-02: Database operations must be transactional where appropriate to avoid inconsistent states.
- NFR-REL-03: Worker and server should be separate processes so the web UI remains responsive if ingestion experiences failures.

6.3 Security & Privacy
- NFR-SEC-01: API keys and secrets must be read from environment variables or a config file excluded from VCS.
- NFR-SEC-02: Sanitize and never execute remote JavaScript. Strip unsafe tags before storing HTML snippets.
- NFR-SEC-03: Provide clear documentation about data stored and where it resides (local SQLite by default).
- NFR-SEC-04: Ensure that any remote LLM outbound traffic is via TLS and that error handling does not leak secrets in logs.

6.4 Portability & Deployment
- NFR-PLT-01: Provide Docker Compose manifests for multi-arch (armv7 for RPi3 and amd64).
- NFR-PLT-02: Use SQLite for local installs; ensure the code is compatible with other SQL backends via `sqlx` where practical.

6.5 Observability & Debuggability
- NFR-OBS-01: Emit structured logs via `tracing`.
- NFR-OBS-02: Expose basic runtime metrics: last ingestion run, number of feeds, queue length, LLM availability.
- NFR-OBS-03: Provide a debug mode for verbose logs and a way to replay ingestion for a feed.

6.6 Maintainability & Extensibility
- NFR-MNT-01: Modular architecture with clear traits/interfaces for LLM, fetcher, parser, storage to allow future replacement or extension.
- NFR-MNT-02: Follow coding guidelines and include unit/integration tests for critical modules.

7. Data Model (high level)
--------------------------
This section gives the principal entities and relationships (not exhaustive).

- users
  - id, name, preferred_language, prefs_json, created_at
- feeds
  - id, user_id, url, site_url, title, last_checked, status, weight
- articles
  - id, canonical_url, title, content_snippet, full_content (nullable), published_at, first_seen_at, canonical_hash
- article_occurrences
  - id, article_id, feed_id, feed_item_id, discovered_at
- summaries
  - id, article_id (nullable for multi-article digest), session_id, summary_text, summary_type (extractive/llm), created_at
- sessions
  - id, user_id, start_at, duration_requested_seconds, digest_summary_id, created_at
- chat_messages
  - id, session_id, author (user/assistant/system), message_text, created_at
- llm_usage (optional)
  - id, session_id, engine, tokens_in, tokens_out, duration_ms, error (nullable)

8. APIs & Interfaces (high level)
---------------------------------
The MVP exposes an HTTP server (Rocket) with endpoints and a WebSocket for chat. High-level endpoints:

- Authentication (initially file-based):
  - POST /login  (basic)
- Feed management:
  - GET /api/v1/feeds
  - POST /api/v1/feeds  (add feed)
  - DELETE /api/v1/feeds/{id}
  - POST /api/v1/feeds/import-opml  (multipart file)
- Status & admin:
  - GET /api/v1/status  (last ingestion, errors)
  - GET /api/v1/config  (read-only displayed)
- Sessions & digests:
  - POST /api/v1/sessions  (start session with requested duration)
  - GET /api/v1/sessions  (list)
  - GET /api/v1/sessions/{id} (fetch transcript + digest)
- Chat websocket:
  - /ws/chat?session_id={id}
- Summaries:
  - GET /api/v1/summaries/{id}
  - POST /api/v1/summaries/generate (admin/test)

Notes:
- All endpoints versioned `/api/v1`.
- Endpoints must return structured JSON and handle errors with standard HTTP statuses.

9. Scheduler & Runtime Behavior
-------------------------------
- The worker runs as a separate binary/process and uses `tokio-cron-scheduler` to schedule ingestion jobs at configured wall-clock times.
- Each ingestion window:
  - discover feeds to check,
  - fetch concurrently with per-domain politeness limits,
  - parse feed items,
  - for each new item, store occurrence, possibly fetch full page if short,
  - after collecting items, run deduplication and scoring pipeline,
  - generate summaries for items above a threshold and persist them,
  - record errors and per-feed metadata.
- Provide a "manual run" admin endpoint to trigger ingestion for debug.

10. LLM Design & Requirements
-----------------------------
- Design an LLM adapter trait:
  - Methods: summarize(items, length_target), semantic_similarity(a,b), extract_keywords(text), embed(text) (optional)
- Requirements:
  - Local-first: If a supported local model is installed and configured, prefer it.
  - Remote fallback: If no local model, call configured remote provider using a secure API key read from env/config.
  - Abstraction must handle timeouts, fallback strategies, and provide usage metrics for debugging.
  - LLM outputs must be sanitized and validated before rendering.

11. Scraping / "Page → RSS" conversion heuristics
------------------------------------------------
- When a site has no feed, attempt to find lists of candidate article links by:
  - Looking for repeated DOM structures (lists, repeated <article> or <li> with link/title/date),
  - Extracting link + title + snippet and canonicalizing to produce a synthetic feed.
- Heuristics:
  - Only extract items that appear "recent" (based on date in page or order) and have a link and title.
  - If the site requires JS to render a list, do not attempt to run JS — mark as "not convertible" (optionally instruct the user).
  - Respect politeness and limit per-site fetches.
- Provide a lightweight "site monitor" that stores the discovered items and treats them like feed items for the pipeline.

12. UX / UI Requirements
------------------------
- The UI aims to be minimal and fast on RPi.
- Session flow:
  - User logs in -> Chat window opens -> System asks for feed sources if none -> user adds feeds or imports OPML.
  - For active system: user clicks "Start session" and inputs duration (minutes). System shows digest (left pane) and chat (right pane) on wide screens; on narrow screens the digest is shown first then accessible via a toggle.
  - Timer is informational: when time is up show unobtrusive notification; the user can continue the conversation beyond the timer.
- Controls:
  - Per-item vote (relevant/not relevant), "save for later", "open source" link.
  - Preference toggles via chat (explicit statements processed).
- Accessibility: basic keyboard navigation and readable fonts.

13. Configuration & Admin
-------------------------
- `config.example.toml` should include:
  - `db.path` (default `data/mynewslens.db`)
  - `schedule.times` (list of HH:MM)
  - `fetch.timeout_seconds`, `fetch.max_size_bytes`
  - `politeness.delay_seconds`, `politeness.max_concurrency_per_domain`
  - `robots_txt.respect` (bool)
  - `llm.adapter` (local|remote|none)
  - `llm.remote.url`, `llm.remote.api_key` (read from env recommended)
  - `logging.level`
- Admin endpoint to view last run logs and ingestion stats (read-only).

14. Testing & Acceptance Criteria
---------------------------------
Testing plan:
- Unit tests for parsing, scoring, deduplication logic.
- Integration tests for:
  - feed ingestion pipeline with mocked HTTP endpoints,
  - OPML import,
  - session start + digest generation with mocked LLM adapter.
- End-to-end test: start server + worker in test mode with sample feeds and ensure a session digest is produced.

Acceptance criteria for MVP:
- A user can configure feeds (manual or OPML), start the worker schedule, and see digests generated on next run.
- A user can start a session requesting X minutes, receive a digest targeted at X/2 reading time, and engage in a chat with stored transcript.
- System runs on RPi 3+ for ingestion tasks without active local LLM; remote LLM integration works when configured.
- Basic feedback (relevant/not relevant) updates preference store and influences subsequent scoring.
- No secrets are present in the repo; configuration example provided.

15. Roadmap (short bullets)
---------------------------
- Deliverable 1: Code skeleton + database migrations + Docker Compose + basic server and worker.
- Deliverable 2: Feed ingestion, OPML import, scraping heuristics for site→feed conversion.
- Deliverable 3: LLM adapter abstraction + remote provider connector for summarization.
- Deliverable 4: Chat UI (WebSocket) + session persistence + feedback loops.
- Deliverable 5: Local LLM support (optional) + semantic deduplication + preference learning improvements.
- Deliverable 6: Full-text search with `tantivy`, mobile notifications, multi-user management.

Appendix A: Constraints & Decisions taken for MVP
-------------------------------------------------
- Use SQLite (`sqlx`) for simplicity and RPi compatibility; keep code compatible with other DBs.
- Rocket chosen for the backend for faster developer iteration.
- `feed-rs` for robust parsing of diverse feed formats.
- `scraper` + heuristics for page extraction; no headless browser or JS execution in the MVP.
- Local LLM support is a feature but optional — remote support included to avoid blocking users on limited hardware.
- Robots.txt respect is configurable (user choice), but politeness (rate-limiting) is enforced.

---

This SPEC is intended to be actionable: each functional requirement should translate into one or more issues/tasks in the backlog. If you'd like, I can:
- split FRs into a prioritized backlog with rough estimates,
- produce a minimal DB migration script skeleton,
- or create the repository skeleton and initial files based on this spec.