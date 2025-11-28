# MyNewsLens — Coding Guidelines

Purpose
-------
This document consolidates conventions, rules and recommendations for contributors to the MyNewsLens project. It is written for an international, open-source audience and focuses on maintainability, readability, reliability and safety — particularly given the project's target of running on resource-constrained platforms (Raspberry Pi 3+) as well as x64 machines.

Apply these guidelines in all repository code, tests, CI, documentation and Docker images unless a PR explicitly documents and justifies a deviation.

Table of contents
-----------------
- Format & linting
- Repository layout
- Naming & style conventions
- Architecture & separation of concerns
- Important interfaces (recommended traits)
- Async, blocking & CPU-heavy work
- Error handling and logging
- Database & migrations
- Networking, scraping and politeness
- Security & sanitization
- Tests and test data
- CI / CD
- Commits, branches and pull requests
- Performance and RPi constraints
- Documentation & docs policy
- Onboarding checklist for new contributors

Format & linting
----------------
- Always run `cargo fmt` before committing. The project enforces canonical Rust formatting.
- Use `clippy` and treat relevant lints as errors in CI:
  - Local command: `cargo clippy --all-targets --all-features -- -D warnings`
  - Address warnings rather than allow them except when explicitly documented.
- Keep `rust-toolchain` pinned to a stable toolchain the project CI uses.
- Lock dependency versions in `Cargo.toml` to specific versions (no `*`).

Repository layout
-----------------
Recommended workspace layout:

- `Cargo.toml` (workspace)
- `server/` — Rocket web server binary crate
- `worker/` — ingestion/cron worker binary crate
- `common/` — shared domain types, DB models, traits and utilities
- `llm/` — LLM adapters (local, remote) and abstractions (optional split)
- `migrations/` — SQLx migrations
- `docker-compose.yml`, `Dockerfile.*`
- `docs/` — architecture, deploy and operational docs
- `tests/` — integration tests (external-like scenarios)
- `examples/` — small runnable examples to illustrate usage

Keep crates small and focused. Prefer moving shared code into `common/` instead of copy/paste.

Naming & style conventions
-------------------------
- Crate and module names: `snake_case`.
- Types (struct/enum/trait): `CamelCase`.
- Functions and method names: `snake_case`.
- Constants: `SCREAMING_SNAKE_CASE`.
- Feature flags: kebab-case like `--features "local-llm"`.
- Prefer descriptive names over short ambiguous ones.
- Keep functions short (∼20–80 lines) and single-responsibility.

Architecture & separation of concerns
-------------------------------------
Follow a modular, hexagonal approach:

- Transport layer (HTTP, WebSocket) lives in `server/` and only handles authentication, request parsing/validation, response formatting and orchestration.
- Domain logic (scoring, deduplication, feed normalization, session management) belongs to `common/`.
- IO-heavy processes such as periodic ingestion, HTML fetching and LLM calls belong to `worker/` or carefully-encapsulated adapters.
- Define clear interfaces (traits) for:
  - LLM adapters
  - HTTP fetcher
  - Feed parser
  - Storage layer (DB repository pattern)
  - HTML extractor

Important interfaces (recommended trait sketches)
-----------------------------------------------
Put trait definitions in `common::traits` and provide one or more implementations.

Example `Llm` trait (conceptual):
`Note: use these as guidance — adapt signatures to the actual codebase.`
- `trait Llm { async fn summarize(&self, req: SummarizeRequest) -> Result<SummarizeResponse>; async fn embed(&self, text: &str) -> Result<Vec<f32>>; }`

Example `Fetcher` trait:
- `trait Fetcher { async fn fetch(&self, url: &Url, opts: &FetchOptions) -> Result<FetchResult>; }`

Example `FeedParser` trait:
- `trait FeedParser { fn parse(&self, bytes: &[u8]) -> Result<FeedDocument>; }`

Design adapters so that:
- Production implementations are in `llm::remote`, `llm::local` and `fetch::reqwest_impl`.
- Tests can inject mocks implementing the same traits.

Async, blocking & CPU-heavy work
--------------------------------
- Use `tokio` as the async runtime.
- Prefer non-blocking operations. Never call blocking APIs in async contexts.
- For CPU-bound or blocking tasks (e.g., local LLM inference, heavy HTML parsing, embedding computations), use `tokio::task::spawn_blocking` or run in a dedicated worker process/binary to avoid blocking the async runtime.
- Use timeouts for external calls (`tokio::time::timeout`) and fail fast when appropriate.
- For heavy tasks that need isolation (LLM inference, large embeddings), prefer running them in a separate process/container.

Error handling and logging
--------------------------
- Return `Result<T, E>` from library functions.
- Use `thiserror` for well-structured crate-level error enums. Use `anyhow` in binaries where ergonomic error propagation is acceptable.
- Do not leak implementation details or secrets in error messages.
- Use `tracing` for structured logging.
  - Log at appropriate levels: `trace` (detailed), `debug` (diagnostic), `info` (high-level events), `warn`, `error`.
  - Avoid logging PII or API keys.
- Include contextual fields in logs for easier debugging (feed id, article id, user id where applicable).

Database & migrations
---------------------
- Use `sqlx` with explicit queries and migrations in `migrations/`.
- Always create a migration for schema changes. Prefer `sqlx migrate add` to create migration files.
- Use transaction boundaries for multi-table changes.
- Configure SQLite pragmas appropriate for the environment (e.g., WAL mode, synchronous setting) but do so explicitly in startup code and document the tradeoffs.
- Pool sizing matters: SQLite with `sqlx` uses a connection pool; keep `max_connections` conservative on RPi (for example 4–10 depending on memory).
- Use prepared statements and parameterized queries to avoid SQL injection.

Networking, scraping and politeness
----------------------------------
- HTTP client: `reqwest` async.
- Feed parsing: `feed-rs`.
- HTML extraction: `scraper` (with `html5ever` if needed).
- Respect politeness:
  - A domain-level concurrency limit and a crawl delay (configurable, default 1s) must be enforced by the fetcher.
  - Respect `robots.txt` should be configurable (default OFF per user-level app requirement, but configurable to ON). However politeness throttling always applies.
- Limit the maximum content size fetched (e.g., 512 KB) and abort oversized responses.
- Set reasonable timeouts for fetches (e.g., 10s default, configurable).
- Follow redirects but detect redirect loops and limit the number of redirects (e.g., 5).

Security & sanitization
-----------------------
- Sanitize all HTML extracted from external sites before storing or rendering. Use a well-tested sanitizer crate (for example `ammonia` for HTML sanitization) and maintain a safe whitelist of tags/attributes if rendered in the UI.
- Never execute JavaScript from scraped pages. Strip scripts and inline event handlers.
- Store LLM keys and other secrets in environment variables or an external secret manager, never in source code.
- Validate user input thoroughly.
- For authentication in the MVP, use simple local config-backed users; still treat passwords/hashes properly (bcrypt/argon2).
- Limit file system exposure in containers. Keep database and persistent storage in controlled volumes.
- Apply "least privilege" to containers and services; do not run as root inside containers.

Tests and test data
-------------------
- Unit tests for pure logic: parsing, scoring, deduplication, small utilities.
- Integration tests for:
  - HTTP endpoints using `rocket::local::asynchronous::Client`.
  - Worker flows with controlled test inputs.
- Network interactions should be mocked in tests:
  - Use crates like `wiremock` or `mockito` to mock HTTP responses rather than relying on external network in CI.
- Keep test fixtures under `tests/fixtures/`.
- For tests that require a DB, use a disposable in-memory SQLite database or a temporary file; ensure migrations are applied in test setup.
- Add CI checks that run the test suite on each PR.

CI / CD
-------
- Use GitHub Actions or similar with pipeline:
  1. `fmt` check: `cargo fmt -- --check`
  2. `clippy` with `-D warnings`
  3. `test` suite
  4. Build artifacts for both `server` and `worker` (multi-arch builds optional in a separate job)
- For release:
  - Build multi-arch Docker images using Docker Buildx for ARMv7 and amd64.
  - Publish artifacts and release notes.
- Fail CI on lint or test failures.

Commits, branches and pull requests
----------------------------------
- Branch naming:
  - Features: `feature/<short-description>`
  - Fixes: `fix/<short-description>`
  - Chores: `chore/<short-description>`
- Commit message style: start with an imperative verb (e.g., "Add feed ingestion scheduler").
- PRs should include:
  - Description of the change
  - Test plan and test coverage
  - Any migration steps
  - Screenshots if UI changes
- Each PR should be reviewed by at least one other person (or another maintainer if contributor is sole developer).
- Small, focused PRs are preferred.

Performance and RPi constraints
-------------------------------
- Assume limited RAM and CPU on RPi 3. Optimize for:
  - Low memory usage
  - Small binary sizes (strip debug symbols in production builds)
  - Avoid big in-memory caches; prefer small persistent caches or bounded caches
- Local LLM inference will likely be expensive — keep this optional in config and use fallback to remote only when allowed by user.
- Monitor memory and CPU usage in dev and tune defaults accordingly.
- Use `spawn_blocking` or a separate worker binary for expensive CPU tasks.

Documentation & docs policy
---------------------------
- Document public crate APIs using `///` doc comments.
- Update `docs/ARCHITECTURE.md` and `docs/DEPLOYMENT.md` when making architecture or deployment changes.
- Provide a `config.example.toml` and clear setup steps in `docs/SETUP.md`.
- Keep the README user-focused (what problem the app solves, who it is for, and how to try it).

Onboarding checklist for new contributors
-----------------------------------------
- [ ] Fork repo and clone
- [ ] Install Rust stable as pinned in `rust-toolchain`
- [ ] Run `cargo fmt`
- [ ] Run `cargo clippy --all-targets --all-features`
- [ ] Run `cargo test`
- [ ] Read `docs/ARCHITECTURE.md` and `docs/SETUP.md`
- [ ] Open PR with small, focused changes and link to related issue

Final notes
-----------
- Treat these guidelines as living. If a rule needs change, propose the change in an issue and discuss it.
- Prioritize safety, user privacy and resource-efficiency. These constraints guide many choices across the codebase (simpler features, optional heavy LLM usage, politeness in scraping, strict sanitization).
- When in doubt, prefer clarity, minimalism and testability.

Thank you for contributing — clear, consistent code and good tests will make MyNewsLens reliable and maintainable for users on both RPi and x64 machines.