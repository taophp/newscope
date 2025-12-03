# Newscope

**Newscope** is a self-hosted, AI-powered personal news assistant. It aggregates RSS feeds, summarizes articles using local or remote LLMs, and provides a conversational interface to explore your news. If you struggle to keep up with multiple blogs, newsletters, and news sites, and you want a focused, time-boxed way to perform high-quality research or daily/weekly monitoring, Newscope is for you. It aggregates sources you care about, prioritizes and de-duplicates information, and produces concise, actionable summaries that you can read and discuss with an AI assistant — all within the time you choose to spend.

License: Affero GPL v3

---

Why this exists — the user problem
- Information overload: dozens of feeds, bookmarks, and sites compete for attention.
- Wasted time: scanning lots of headlines and low-value posts consumes energy.
- Poor signal extraction: important cross-posted information or subtle trends get missed.
- No integrated, time-boxed workflow: users lack a tool to constrain the time they spend while giving high-value results.

Who it's for
- Individual knowledge workers (developers, researchers, product managers) who want a compact and repeatable way to keep up with domain-specific news.
- People who prefer a private or self-hosted solution (runs on a Raspberry Pi or x86 host).
- Privacy-conscious users who favor local LLM options but want the flexibility to use remote models.

What Newscope gives you
- Aggregation: collect RSS/OPML feeds and convert site pages to feed-like items when needed.
- Prioritization: rank items by relevance, novelty, redundancy across feeds, and your learned preferences.
- Summarization: concise, hierarchical summaries tailored to the time you allocate (e.g., “10 minutes of reading”).
- Conversational exploration: a chat-based interface to drill down into topics, refine preferences, and follow up on summaries.
- Archival: each session is timestamped and stored so you can resume or revisit discussions later.
- Configurable privacy/AI backend: use a local model when available, or a remote model as fallback.

Key design principles
- Time-boxed: the app's core UX ensures you spend only the time you allocated.
- Assistive AI: the assistant summarizes and helps you explore, and learns preferences from the conversation.
- Self-hostable: designed to run on modest hardware (Raspberry Pi 3+) and on x86.
- Single-binary, modular design: the application ships as a single Rust executable that runs both the HTTP server (Rocket) and the background worker(s) inside the same process and tokio runtime. Logical separation is preserved via library modules (ingestion, indexing, LLM adapter, UI), but deployment and orchestration are simplified to a single container or service.
- Respectful scraping: politeness enforced, robots.txt optional, and no embedded remote JS execution from scraped pages.

Minimum Viable Product (MVP) — what you'll get first
- Self-hosted server written in Rust using `Rocket`.
- Feed ingestion using `feed-rs` and HTML extraction via `scraper`.
- Worker scheduler (app-level scheduling) that runs at default times: 05:00, 11:00, 17:00, 23:00 local time.
- Basic semantic deduplication (hash + LLM-assisted pass when available).
- Summaries produced by an LLM abstraction (supports local models or remote providers; remote providers not limited to a single vendor).
- Simple web UI (JavaScript) with:
  - Login via configuration-defined users,
  - Chat interface (WebSocket or SSE),
  - Summary display alongside a timer (informational),
  - OPML import and manual feed management.
- SQLite storage via `sqlx` with migrations.
- Docker Compose configuration for multi-arch deployment (Raspberry Pi and x86).

Files that accompany this README
- `ROADMAP.md` — prioritized milestones and longer-term plan.
- `SPEC.md` — detailed technical specification and non-functional requirements.
- `CODING_GUIDELINES.md` — coding standards and development rules.
(These files live in the repo root; see them for design and contributor guidance.)

Quick usage summary (for evaluation)
1. Install and configure Newscope (see `docs/SETUP.md` or `SPEC.md`).
2. Add feeds or import an OPML file.
3. Run the single Newscope executable (or start via Docker Compose). The executable runs both the web server and the background worker inside the same process and tokio runtime by default. Configuration can be provided via `config.toml` and environment variables.
   - CLI flags exist to control runtime behavior (examples):
     - `--no-worker` : launch only the HTTP server and disable background ingestion tasks.
     - `--worker-only` : run ingestion and worker tasks without binding the HTTP server.
     - `--config /path/to/config.toml` : use a custom configuration file.
4. The worker runs at configured times and ingests new items (see default schedule in `config.example.toml`).
5. Start a timed session through the UI. The assistant generates a concise summary (designed to be readable in half the time you selected) and a chat opens for follow-up. The UI displays an informational timer and preserves the session archive for later review.
6. During the chat, provide feedback inline (likes/dislikes, or explicit preferences). The assistant learns from this to reduce irrelevant future items.

Security & privacy notes (short)
- API keys for remote LLMs are read from environment variables and never committed.
- Scraped HTML is stored in MarkDown; scripts from remote pages are never executed.
- Media assets (images/videos) are not downloaded by default — the summary always links back to the source.

Getting involved
- Open source contribution is welcome. See `CODING_GUIDELINES.md` and `SPEC.md` for how to contribute.
- If you want to test local LLM features on low-powered hardware, start with small models or remote fallbacks.

---

## Quick Start

### First Time Setup

1. **Configure the application (Optional)**:
   The application works out-of-the-box with `config.default.toml`.
   To customize settings (e.g., add users or change LLM), create `config.toml`:
   ```bash
   # Optional: Override defaults
   cp config.default.toml config.toml
   # Edit config.toml
   ```

2. **Set up LLM (for chat features)**:
   
   **Option A: Local with Ollama** (Default)
   ```bash
   # Install Ollama
   curl -fsSL https://ollama.com/install.sh | sh
   
   # Start Ollama
   ollama serve
   
   # Pull the default model
   ollama pull llama3.2:3b
   
   # Set environment variable (Required even if dummy)
   export OLLAMA_API_KEY="dummy"
   ```

   **Option B: OpenAI** (requires config override)
   Create `config.toml` and add:
   ```toml
   [llm]
   adapter = "remote"
   
   [llm.remote]
   api_url = "https://api.openai.com/v1/chat/completions"
   api_key_env = "OPENAI_API_KEY"
   model = "gpt-4o-mini"
   ```
   Then export your key:
   ```bash
   export OPENAI_API_KEY="sk-..."
   ```

3. **Run the server**:
   ```bash
   # Uses config.default.toml + config.toml (if present)
   cargo run --bin newscope
   ```

4. **Open your browser**: http://localhost:8000

5. **Create your first account**:
   - Click "Create an account"
   - Enter username, display name, and password
   - Click "Register"
   
   You'll be automatically logged in!

6. **Add your first feed**:
   - Click "+ Add Feed"
   - Enter a feed URL (e.g., `http://rss.cnn.com/rss/edition.rss`)
   - Click "Add Feed"

7. **Start a chat session**:
   - Click "+ New Session"
   - Choose duration (5-60 minutes)
   - Click "Start Session"
   - Chat with your AI assistant about the news!

### Pre-configured Users (Optional)

You can also define users in `config.toml`:

```toml
[[users]]
username = "demo"
display_name = "Demo User"
```

Note: Pre-configured users without passwords can only be used if you set a password through the registration UI first, or you manually hash a password and add it to the config (not recommended).

**Recommendation**: Just use the registration form - it's the easiest way to get started!

---

Thanks for checking out Newscope — built to make focused, AI-assisted research and daily monitoring simple, private, and time-efficient.
