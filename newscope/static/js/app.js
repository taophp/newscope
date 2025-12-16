// Main App Logic

const App = {
  currentUser: null,
  chatManager: new ChatManager(),
  currentSession: null,

  init() {
    this.userHasInteracted = false;
    // Hide loading, check auth
    document.getElementById("loading-screen").classList.add("hidden");

    // Check if already logged in
    const token = localStorage.getItem("mnl_token");
    const userId = localStorage.getItem("mnl_user_id");

    if (token && userId) {
      this.currentUser = { id: parseInt(userId), token };
      this.showApp();
    } else {
      this.showAuth();
    }

    this.setupEventListeners();
  },

  setupEventListeners() {
    // Auth toggle
    document.getElementById("show-register").addEventListener("click", (e) => {
      e.preventDefault();
      document.getElementById("login-form").classList.add("hidden");
      document.getElementById("register-form").classList.remove("hidden");
    });

    document.getElementById("show-login").addEventListener("click", (e) => {
      e.preventDefault();
      document.getElementById("register-form").classList.add("hidden");
      document.getElementById("login-form").classList.remove("hidden");
    });

    // Auth forms
    document
      .getElementById("form-login")
      .addEventListener("submit", (e) => this.handleLogin(e));
    document
      .getElementById("form-register")
      .addEventListener("submit", (e) => this.handleRegister(e));

    // Logout
    document
      .getElementById("btn-logout")
      .addEventListener("click", () => this.logout());

    // Modals
    document
      .getElementById("btn-add-feed")
      .addEventListener("click", () => this.openModal("modal-add-feed"));
    document.querySelectorAll(".modal-close").forEach((btn) => {
      btn.addEventListener("click", (e) =>
        this.closeModal(e.target.closest(".modal").id),
      );
    });

    // Feed form
    document
      .getElementById("form-add-feed")
      .addEventListener("submit", (e) => this.handleAddFeed(e));

    // Session
    document
      .getElementById("btn-new-session")
      .addEventListener("click", () => this.openModal("modal-new-session"));
    document
      .getElementById("btn-welcome-session")
      .addEventListener("click", () => this.openModal("modal-new-session"));
    document
      .getElementById("form-new-session")
      .addEventListener("submit", (e) => this.handleNewSession(e));

    // Session duration slider
    const slider = document.getElementById("session-duration");
    slider.addEventListener("input", (e) => {
      document.getElementById("duration-value").textContent = e.target.value;
    });

    // Chat
    document
      .getElementById("btn-close-chat")
      .addEventListener("click", () => this.closeChat());
    document
      .getElementById("btn-send")
      .addEventListener("click", () => this.sendMessage());
    document
      .getElementById("message-input")
      .addEventListener("keydown", (e) => {
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          this.sendMessage();
        }
      });

    // OPDS Import
    document
      .getElementById("btn-import-opds")
      .addEventListener("click", () => this.openOPDSModal());
    document
      .getElementById("btn-opds-import")
      .addEventListener("click", () => this.handleOPDSImport());
  },

  showAuth() {
    document.getElementById("auth-view").classList.remove("hidden");
    document.getElementById("app-view").classList.add("hidden");
  },

  showApp() {
    document.getElementById("auth-view").classList.add("hidden");
    document.getElementById("app-view").classList.remove("hidden");
    this.loadFeeds();
    this.loadSessions();
  },

  async handleLogin(e) {
    e.preventDefault();
    const username = document.getElementById("login-username").value;
    const password = document.getElementById("login-password").value;

    try {
      const data = await API.login(username, password);
      localStorage.setItem("mnl_token", data.token);
      localStorage.setItem("mnl_user_id", data.user_id);
      this.currentUser = { id: data.user_id, token: data.token };
      this.showApp();
    } catch (error) {
      alert("Login failed: " + error.message);
    }
  },

  async handleRegister(e) {
    e.preventDefault();
    const username = document.getElementById("reg-username").value;
    const displayName = document.getElementById("reg-display").value;
    const password = document.getElementById("reg-password").value;

    try {
      const data = await API.register(username, displayName, password);
      localStorage.setItem("mnl_token", data.token);
      localStorage.setItem("mnl_user_id", data.user_id);
      this.currentUser = { id: data.user_id, token: data.token };
      this.showApp();
    } catch (error) {
      alert("Registration failed: " + error.message);
    }
  },

  async logout() {
    // Attempt soft logout: revoke token on server before clearing local state.
    try {
      await API.logout().catch((e) => {
        // Non-fatal: log and continue clearing client state
        console.warn("Failed to revoke token on server during logout:", e);
      });
    } catch (e) {
      console.warn("Unexpected error during logout revoke:", e);
    }

    // Clear auth tokens and local user state (defensive: API.logout may have cleared already)
    localStorage.removeItem("mnl_token");
    localStorage.removeItem("mnl_user_id");
    this.currentUser = null;

    // Close any active chat/websocket and stop timers
    if (this.chatManager) {
      try {
        this.chatManager.disconnect();
      } catch (e) {
        console.warn("Error while disconnecting chat manager:", e);
      }
    }

    // Clear chat UI state to avoid leaking data between users
    const chatMessages = document.getElementById("chat-messages");
    if (chatMessages) chatMessages.innerHTML = "";
    const messageInput = document.getElementById("message-input");
    if (messageInput) messageInput.value = "";

    // Reset auth forms: show login, hide register, and clear inputs
    const loginForm = document.getElementById("login-form");
    const registerForm = document.getElementById("register-form");
    if (registerForm) registerForm.classList.add("hidden");
    if (loginForm) loginForm.classList.remove("hidden");

    const loginUsername = document.getElementById("login-username");
    const loginPassword = document.getElementById("login-password");
    if (loginUsername) loginUsername.value = "";
    if (loginPassword) loginPassword.value = "";

    // Focus username field so the user can quickly re-login
    if (loginUsername) {
      try {
        loginUsername.focus();
      } catch (e) {
        // ignore focus errors in environments where it's not possible
      }
    }

    this.showAuth();
  },

  async loadFeeds() {
    try {
      const feeds = await API.getFeeds(this.currentUser.id);
      this.renderFeeds(feeds);
    } catch (error) {
      console.error("Failed to load feeds:", error);
    }
  },

  renderFeeds(feeds) {
    const container = document.getElementById("feed-list");
    if (!feeds || feeds.length === 0) {
      container.innerHTML = '<p class="empty-state">No feeds yet</p>';
      return;
    }

    container.innerHTML = feeds
      .map(
        (feed) => `
            <div class="feed-item">
                <div class="feed-content">
                    <div class="feed-title">${feed.title || "Untitled Feed"}</div>
                    <div class="feed-url">${feed.url}</div>
                </div>
                <button class="btn-icon btn-refresh" data-feed-id="${feed.id}" title="Refresh feed">
                    🔄
                </button>
            </div>
        `,
      )
      .join("");

    // Add click handlers for refresh buttons
    document.querySelectorAll(".btn-refresh").forEach((btn) => {
      btn.addEventListener("click", (e) => {
        e.stopPropagation();
        this.handleRefreshFeed(parseInt(btn.dataset.feedId), btn);
      });
    });
  },

  async handleRefreshFeed(feedId, button) {
    button.disabled = true;
    button.textContent = "⏳";

    try {
      await API.triggerFetch(feedId);
      // Show success feedback
      button.textContent = "✓";
      setTimeout(() => {
        button.textContent = "🔄";
        button.disabled = false;
      }, 2000);
    } catch (error) {
      console.error("Failed to refresh feed:", error);
      button.textContent = "✗";
      setTimeout(() => {
        button.textContent = "🔄";
        button.disabled = false;
      }, 2000);
    }
  },

  async loadSessions() {
    try {
      const sessions = await API.getSessions(this.currentUser.id);
      this.renderSessions(sessions);
    } catch (error) {
      console.error("Failed to load sessions:", error);
    }
  },

  renderSessions(sessions) {
    const container = document.getElementById("session-list");
    if (!sessions || sessions.length === 0) {
      container.innerHTML = '<p class="empty-state">No sessions</p>';
      return;
    }

    container.innerHTML = sessions
      .map((session) => {
        const date = new Date(session.start_at * 1000);
        const dateStr = date.toLocaleDateString(undefined, {
          weekday: "short",
          month: "short",
          day: "numeric",
        });
        const timeStr = date.toLocaleTimeString(undefined, {
          hour: "2-digit",
          minute: "2-digit",
        });
        const title = session.title || `Session #${session.id}`;

        return `
            <div class="session-item" data-session-id="${session.id}">
                <div class="session-info">
                    <div class="feed-title" title="${title}">${title}</div>
                    <div class="feed-url">${dateStr} ${timeStr}</div>
                </div>
                <button class="btn-icon btn-rename-session" title="Rename Session">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                        <path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"></path>
                        <path d="M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"></path>
                    </svg>
                </button>
            </div>
            `;
      })
      .join("");

    // Add click handlers
    document.querySelectorAll(".session-item").forEach((item) => {
      item.addEventListener("click", (e) => {
        // Ignore if clicking rename button
        if (e.target.closest(".btn-rename-session")) return;

        const sessionId = parseInt(item.dataset.sessionId);
        this.loadSessionHistory(sessionId);
      });

      // Rename handler
      const renameBtn = item.querySelector(".btn-rename-session");
      if (renameBtn) {
        renameBtn.addEventListener("click", (e) => {
          e.stopPropagation();
          const sessionId = parseInt(item.dataset.sessionId);
          const currentTitle = item.querySelector(".feed-title").textContent;
          this.renameSession(sessionId, currentTitle);
        });
      }
    });
  },

  async renameSession(sessionId, currentTitle) {
    const newTitle = prompt("Enter new session name:", currentTitle);
    if (newTitle && newTitle.trim() !== "") {
      try {
        await API.updateSession(sessionId, newTitle.trim());
        this.loadSessions(); // Reload list
      } catch (error) {
        alert("Failed to rename session: " + error.message);
      }
    }
  },

  async loadSessionHistory(sessionId) {
    try {
      // Close sidebar on mobile if needed (optional)

      // Fetch full session with messages
      const data = await API.getSession(sessionId);
      this.currentSession = data.session;

      // Clear current chat
      const container = document.getElementById("chat-messages");
      container.innerHTML = "";

      // Replay messages
      if (data.messages && data.messages.length > 0) {
        data.messages.forEach((msg) => {
          this.addMessage(msg.author, msg.message);
        });
      } else {
        this.addMessage("assistant", "This session has no history.");
      }

      // Re-initialize chat manager with this session
      // Note: ChatManager usually creates a new session. We might need to adapt it
      // to resume an existing one. For now, we just display history.
      // If the user sends a message, it should ideally append to this session.
      // Let's update ChatManager to support resuming.
      this.chatManager.resumeSession(sessionId);
    } catch (error) {
      console.error("Failed to load session history:", error);
      alert("Failed to load session history.");
    }
  },

  async handleAddFeed(e) {
    e.preventDefault();
    const url = document.getElementById("feed-url").value;
    const title = document.getElementById("feed-title").value;

    try {
      await API.createFeed(url, title, this.currentUser.id);
      this.closeModal("modal-add-feed");
      document.getElementById("form-add-feed").reset();
      this.loadFeeds();
    } catch (error) {
      alert("Failed to add feed: " + error.message);
    }
  },

  async handleNewSession(e) {
    e.preventDefault();
    const duration = parseInt(
      document.getElementById("session-duration").value,
    );
    const durationSeconds = duration * 60;

    try {
      const data = await API.createSession(
        this.currentUser.id,
        durationSeconds,
      );
      this.currentSession = data;
      this.closeModal("modal-new-session");
      this.openChat(data.id, durationSeconds);
    } catch (error) {
      alert("Failed to start session: " + error.message);
    }
  },

  async openChat(sessionId, durationSeconds = null) {
    // Added durationSeconds parameter with default null
    try {
      const data = await API.getSession(sessionId);
      this.currentSession = data.session;

      // If durationSeconds was not passed, get it from the session data
      if (durationSeconds === null) {
        durationSeconds = this.currentSession.duration;
      }

      // Set session duration on chat manager
      this.chatManager.setSessionDuration(durationSeconds);

      // Show chat screen
      document.getElementById("welcome-screen").classList.add("hidden");
      document.getElementById("chat-screen").classList.remove("hidden");
      document.getElementById("chat-session-id").textContent = sessionId;

      // Load history
      document.getElementById("chat-messages").innerHTML = "";
      if (data.messages && data.messages.length > 0) {
        data.messages.forEach((msg) =>
          this.addMessage(msg.author, msg.message),
        );
      }

      // Request notification permission
      if ("Notification" in window && Notification.permission === "default") {
        await Notification.requestPermission();
      }

      // Set handlers before connecting to avoid missing early messages
      this.chatManager.onMessage = (data) => this.handleChatMessage(data);
      this.chatManager.onStatus = (status) => this.updateChatStatus(status);
      // Connect WebSocket
      this.chatManager.connect(sessionId);
    } catch (error) {
      alert("Failed to open session: " + error.message);
    }
  },

  closeChat() {
    this.chatManager.disconnect();
    this.currentSession = null;
    document.getElementById("chat-screen").classList.add("hidden");
    document.getElementById("welcome-screen").classList.remove("hidden");
  },

  sendMessage() {
    const input = document.getElementById("message-input");
    const message = input.value.trim();

    if (!message) return;

    this.addMessage("user", message);
    this.showThinking(); // Show thinking indicator
    this.chatManager.send(message);
    input.value = "";
  },

  handleChatMessage(data) {
    // Handle different message types from WebSocket
    if (data.type === "progress") {
      // Show progress indicator with status message
      this.updateProgress(data.message || "Processing...");
    } else if (data.type === "news_item" || data.type === "news_card") {
      // Render a News Card (support both legacy 'news_item' and new 'news_card')
      this.hideProgress();
      this.hideThinking();
      const article = data.article;
      const card = this.renderNewsCard(article);
      const container = document.getElementById("chat-messages");

      // Check if last element is a news container
      let feedContainer = container.lastElementChild;
      if (
        !feedContainer ||
        !feedContainer.classList.contains("news-feed-container")
      ) {
        feedContainer = document.createElement("div");
        feedContainer.className = "news-feed-container";
        // Layout cards side-by-side with wrapping and gap
        feedContainer.style.display = "flex";
        feedContainer.style.flexWrap = "wrap";
        feedContainer.style.gap = "20px";
        feedContainer.style.alignItems = "flex-start";
        container.appendChild(feedContainer);
      }

      feedContainer.appendChild(card);
      // Removed auto-scroll to allow user to read previous cards


      // Send system notification if page is hidden (only for the first card to avoid spam?)
      if (
        document.hidden &&
        "Notification" in window &&
        Notification.permission === "granted"
      ) {
        // Debounce notification?
      }
    } else if (data.type === "news_card_update") {
      // Update an existing card with refined content (or append if missing)
      this.hideProgress();
      this.hideThinking();
      const article = data.article;
      const container = document.getElementById("chat-messages");

      // Find existing card by data-article-id
      const existing = container.querySelector(
        `.news-card[data-article-id="${article.id}"]`,
      );
      const updatedCard = this.renderNewsCard(article);

      if (existing) {
        existing.replaceWith(updatedCard);
      } else {
        // Append to feed container if not present
        let feedContainer = container.lastElementChild;
        if (
          !feedContainer ||
          !feedContainer.classList.contains("news-feed-container")
        ) {
          feedContainer = document.createElement("div");
          feedContainer.className = "news-feed-container";
          // Ensure updated cards use the same horizontal layout
          feedContainer.style.display = "flex";
          feedContainer.style.flexWrap = "wrap";
          feedContainer.style.gap = "20px";
          feedContainer.style.alignItems = "flex-start";
          container.appendChild(feedContainer);
        }
        feedContainer.appendChild(updatedCard);
      }
      // Removed auto-scroll
    } else if (data.type === "message" && data.content) {
      // Hide progress and show new message from server
      this.hideProgress();
      this.hideThinking(); // Hide thinking indicator
      this.addMessage("assistant", data.content, data.sources || null);

      // Send system notification if page is hidden
      if (
        document.hidden &&
        "Notification" in window &&
        Notification.permission === "granted"
      ) {
        new Notification("Newscope", {
          body: "Your press review is ready!",
          icon: "/static/favicon.ico", // Assuming you have one, or remove if not
        });
      }
    } else if (data.type === "history") {
      // Chat history replay
      this.addMessage(
        data.role === "user" ? "user" : "assistant",
        data.content,
      );
    } else if (
      data.type === "message" &&
      data.author === "assistant" &&
      data.message
    ) {
      // Legacy format support
      this.hideProgress();
      this.addMessage("assistant", data.message);
    }
  },

  updateProgress(message) {
    const indicator = document.getElementById("progress-indicator");
    const details = indicator.querySelector(".progress-details");
    indicator.classList.remove("hidden");
    details.textContent = message;
  },

  hideProgress() {
    const indicator = document.getElementById("progress-indicator");
    indicator.classList.add("hidden");
  },

  showThinking() {
    const container = document.getElementById("chat-messages");
    // Remove existing thinking bubble if any
    this.hideThinking();

    const thinkingDiv = document.createElement("div");
    thinkingDiv.id = "thinking-indicator";
    thinkingDiv.className = "message assistant";
    thinkingDiv.innerHTML = `
            <div class="avatar">A</div>
            <div class="thinking-bubble">
                <div class="thinking-dot"></div>
                <div class="thinking-dot"></div>
                <div class="thinking-dot"></div>
            </div>
        `;
    container.appendChild(thinkingDiv);
    container.scrollTop = container.scrollHeight;
  },

  hideThinking() {
    const existing = document.getElementById("thinking-indicator");
    if (existing) {
      existing.remove();
    }
  },

  addMessage(author, text, sources = null) {
    const container = document.getElementById("chat-messages");
    const messageDiv = document.createElement("div");
    messageDiv.className = `message ${author}`;

    const avatar = author === "user" ? "U" : "A";

    // Render Markdown for assistant, escape HTML for user
    let content;
    let usedSources = new Set();

    if (author === "assistant") {
      // Process inline sources BEFORE markdown parsing
      // Look for [Source Name] patterns
      let processedText = text;

      if (sources && sources.length > 0) {
        // Create a map for quick lookup
        const sourceMap = new Map();
        sources.forEach((s) => {
          if (s.source) sourceMap.set(s.source.toLowerCase(), s);
        });

        // Regex to find [Source Name]
        // We use a replacer function to substitute with HTML
        processedText = text.replace(/\[(.*?)\]/g, (match, sourceName) => {
          const source = sourceMap.get(sourceName.toLowerCase());
          if (source) {
            usedSources.add(source);
            return this.renderSourceItem(source, true); // true = inline style
          }
          return match; // Keep original if not found
        });
      }

      if (window.marked) {
        content = marked.parse(processedText);
      } else {
        content = this.escapeHtml(processedText);
      }
    } else {
      content = this.escapeHtml(text);
    }

    // Add remaining sources at the bottom if present
    let sourcesHtml = "";
    if (sources && sources.length > 0) {
      // Filter out sources that were already rendered inline
      const remainingSources = sources.filter((s) => !usedSources.has(s));

      if (remainingSources.length > 0) {
        sourcesHtml = this.renderSources(remainingSources);
      }
    }

    messageDiv.innerHTML = `
            <div class="avatar">${avatar}</div>
            <div class="message-content">
                ${content}
                ${sourcesHtml}
            </div>
        `;

    container.appendChild(messageDiv);

    // Auto-scroll logic:
    // Only scroll if the user has actively interacted with the chat.
    // This prevents the view from jumping to the bottom while the initial 
    // press review (cards + completion msg) is loading, allowing the user 
    // to read cards at their own pace.
    if (author === "user") {
      this.setupIntersectionObserver();
      this.userHasInteracted = false; // Track if user has started chatting

      // Check for active session in URL
      const urlParams = new URLSearchParams(window.location.search);
    }

    if (this.userHasInteracted) {
      container.scrollTop = container.scrollHeight;
    }
  },

  renderSourceItem(s, inline = false) {
    const domain = this.extractDomain(s.url);
    const faviconUrl = `${domain}/favicon.ico`;
    const sourceName = s.source || "Unknown";

    const inlineClass = inline ? "source-inline" : "";
    const style = inline
      ? "display: inline-flex; vertical-align: middle; margin: 0 2px;"
      : "";

    return `
            <a href="${s.url}" target="_blank" rel="noopener noreferrer" class="source-item ${inlineClass}" style="${style}">
                <img src="${faviconUrl}"
                        alt=""
                        class="source-icon"
                        onerror="this.style.display='none'; this.nextElementSibling.style.display='inline';">
                <span class="source-icon-fallback" style="display:none;">📰</span>
                <span class="source-info">
                    <span class="source-name" style="font-weight:600;">${sourceName}</span>
                    ${!inline ? `<span class="source-sep" style="opacity:0.5;">•</span><span class="source-title">${this.truncate(s.title, 40)}</span>` : ""}
                </span>
            </a>
        `;
  },

  renderSources(sources) {
    // Sort by score (relevance)
    sources.sort((a, b) => b.score - a.score);

    return `
            <div class="sources-container">
                <div class="sources-label">Other Sources:</div>
                <div class="sources-list">
                    ${sources.map((s) => this.renderSourceItem(s)).join("")}
                </div>
            </div>
        `;
  },

  extractDomain(url) {
    try {
      const urlObj = new URL(url);
      return `${urlObj.protocol}//${urlObj.hostname}`;
    } catch {
      return "";
    }
  },

  truncate(text, maxLength) {
    return text.length > maxLength
      ? text.substring(0, maxLength) + "..."
      : text;
  },

  updateChatStatus(status) {
    const statusEl = document.getElementById("chat-status");
    statusEl.textContent =
      status === "connected" ? "Connected" : "Disconnected";
    statusEl.style.color =
      status === "connected" ? "var(--success)" : "var(--error)";
  },

  openModal(modalId) {
    document.getElementById(modalId).classList.remove("hidden");
  },

  closeModal(modalId) {
    document.getElementById(modalId).classList.add("hidden");
  },

  escapeHtml(text) {
    const div = document.createElement("div");
    div.textContent = text;
    return div.innerHTML;
  },

  openOPDSModal() {
    // Reset modal state
    document.getElementById("opds-file-input").value = "";
    document.getElementById("opds-progress").classList.add("hidden");
    document.getElementById("opds-results").classList.add("hidden");
    document.getElementById("opds-modal").classList.remove("hidden");
  },

  async handleOPDSImport() {
    const fileInput = document.getElementById("opds-file-input");
    const file = fileInput.files[0];

    if (!file) {
      alert("Please select an OPDS file");
      return;
    }

    // Show progress indicator
    document.getElementById("opds-progress").classList.remove("hidden");
    document.getElementById("opds-results").classList.add("hidden");
    document.getElementById("opds-progress-details").textContent =
      `Uploading ${file.name}...`;

    try {
      const formData = new FormData();
      formData.append("file", file);

      const response = await fetch(
        `/api/v1/feeds/import/opml?user_id=${this.currentUser.id}`,
        {
          method: "POST",
          body: file, // Send raw file data
        },
      );

      if (!response.ok) {
        throw new Error(`Import failed: ${response.statusText}`);
      }

      const result = await response.json();

      // Hide progress, show results
      document.getElementById("opds-progress").classList.add("hidden");
      document.getElementById("opds-results").classList.remove("hidden");

      // Update stats
      document.getElementById("opds-stats-added").textContent = result.added;
      document.getElementById("opds-stats-duplicates").textContent =
        result.duplicates;
      document.getElementById("opds-stats-errors").textContent =
        result.errors.length;

      // Show errors if any
      const errorList = document.getElementById("opds-error-list");
      if (result.errors.length > 0) {
        errorList.classList.remove("hidden");
        errorList.innerHTML =
          "<h4>Errors:</h4><ul>" +
          result.errors.map((e) => `<li>${this.escapeHtml(e)}</li>`).join("") +
          "</ul>";
      } else {
        errorList.classList.add("hidden");
      }

      // Refresh feed list if any feeds were added
      if (result.added > 0) {
        setTimeout(() => {
          this.loadFeeds();
        }, 1000);
      }
    } catch (error) {
      console.error("OPDS import error:", error);
      document.getElementById("opds-progress").classList.add("hidden");
      alert(`Import failed: ${error.message}`);
    }
  },

  renderNewsCard(article) {
    // Create card container
    const card = document.createElement("div");
    card.className = "news-card";
    card.style.boxSizing = "border-box";
    // Set card flex basis so multiple cards can appear side-by-side
    card.style.flex = "0 1 320px";
    card.style.maxWidth = "calc(35em + 40px)";
    card.style.width = "100%";

    // Attach article id for updates
    if (article && article.id !== undefined && article.id !== null) {
      card.setAttribute("data-article-id", String(article.id));
    }

    // Set language attribute to help user agents and screen readers.
    // Prefer explicit article.lang, otherwise use browser primary language.
    try {
      const browserLang = (
        navigator.language ||
        navigator.userLanguage ||
        "en"
      ).split("-")[0];
      card.lang = article && article.lang ? article.lang : browserLang;
    } catch (e) {
      card.lang = article && article.lang ? article.lang : "en";
    }

    // Header
    const header = document.createElement("div");
    header.className = "card-header";
    const titleText = article && article.title ? article.title : "";
    // Keep title safe: render as plain text (no markdown for title) to avoid layout surprises
    header.innerHTML = `
            <h3 class="card-title">${this.escapeHtml(titleText)}</h3>
            <!-- <span class="card-theme">${this.escapeHtml(article.theme || "Actualité")}</span> -->
        `;
    card.appendChild(header);

    // Content
    const content = document.createElement("div");
    content.className = "card-content";
    // Render markdown in the summary if a markdown parser is available (marked included in index.html)
    const summaryText = article && article.summary ? article.summary : "";
    if (window.marked) {
      try {
        content.innerHTML = marked.parse(summaryText);
      } catch (e) {
        content.innerHTML = `<p>${this.escapeHtml(summaryText)}</p>`;
      }
    } else {
      content.innerHTML = `<p>${this.escapeHtml(summaryText)}</p>`;
    }
    card.appendChild(content);

    // Footer
    const footer = document.createElement("div");
    footer.className = "card-footer";

    // Source Icons (Left)
    const sourceIcons = document.createElement("div");
    sourceIcons.className = "source-icons";
    // Support either article.sources (array) or single article.source
    const sourcesArr = Array.isArray(article.sources)
      ? article.sources
      : article.source
        ? [article.source]
        : [];

    if (sourcesArr && Array.isArray(sourcesArr)) {
      sourcesArr.forEach((source) => {
        try {
          // Use source URL if available, otherwise fallback to article URL
          const urlToUse = source.url || article.url;

          // Try to get domain for favicon
          let domain = urlToUse ? new URL(urlToUse).hostname : "";

          if (domain) {
            const link = document.createElement("a");
            link.href = urlToUse;
            link.target = "_blank";
            link.rel = "noopener noreferrer";

            const img = document.createElement("img");
            // Use direct favicon from the site root to avoid third-party services (Google).
            // This is a naive approach but respects privacy.
            img.src = `${new URL(urlToUse).protocol}//${domain}/favicon.ico`;
            img.className = "source-icon";
            img.title = source.name || domain;

            link.appendChild(img);
            sourceIcons.appendChild(link);
          } else if (source && source.name) {
            // Fallback: name-only source
            const span = document.createElement("span");
            span.className = "source-fallback";
            span.textContent = source.name;
            sourceIcons.appendChild(span);
          }
        } catch (e) {
          // Ignore bad URLs
          console.warn("Error rendering source icon:", e);
        }
      });
    }
    footer.appendChild(sourceIcons);

    // Actions (Right)
    const actions = document.createElement("div");
    actions.className = "card-actions";

    // Rating Stars
    const ratingDiv = document.createElement("div");
    ratingDiv.className = "rating-stars";
    // 5 to 1 for row-reverse logic
    for (let i = 5; i >= 1; i--) {
      const star = document.createElement("span");
      star.className = "star";
      star.textContent = "★";
      star.dataset.value = i;
      star.onclick = () => {
        console.log(`Rated ${i} stars for article ${article.id}`);
        // Send rating to backend
        if (this.chatManager) {
          // Keep existing API shape but send a plain object via ChatManager
          try {
            this.chatManager.send(
              JSON.stringify({
                type: "rate",
                article_id: article.id,
                rating: i,
              }),
            );
          } catch (e) {
            // fallback: send simple message
            this.chatManager.send(`rate:${article.id}:${i}`);
          }
        }
        // Visual feedback: update stars
        const stars = ratingDiv.querySelectorAll(".star");
        stars.forEach((s) => {
          if (parseInt(s.dataset.value) <= i) {
            s.classList.add("active");
          } else {
            s.classList.remove("active");
          }
        });
      };
      ratingDiv.appendChild(star);
    }
    actions.appendChild(ratingDiv);

    // Learn More Button
    const btnLearnMore = document.createElement("button");
    btnLearnMore.className = "btn-learn-more";
    btnLearnMore.innerHTML = `
            <span>En savoir plus</span>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M9 18l6-6-6-6"/>
            </svg>
        `;
    btnLearnMore.onclick = () => {
      const query = `Dis-m'en plus sur : ${article.title}`;
      this.addMessage("user", query);
      this.showThinking();
      this.chatManager.send(query);
    };
    actions.appendChild(btnLearnMore);

    footer.appendChild(actions);
    card.appendChild(footer);

    return card;
  },
};

// Initialize app when DOM is ready
document.addEventListener("DOMContentLoaded", () => App.init());

// Global helper for collapsible sections
function toggleSection(header) {
  header.classList.toggle("collapsed");
  const content = header.nextElementSibling;
  content.classList.toggle("collapsed");
}
