// Main App Logic

const App = {
    currentUser: null,
    chatManager: new ChatManager(),
    currentSession: null,

    init() {
        // Hide loading, check auth
        document.getElementById('loading-screen').classList.add('hidden');

        // Check if already logged in
        const token = localStorage.getItem('mnl_token');
        const userId = localStorage.getItem('mnl_user_id');

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
        document.getElementById('show-register').addEventListener('click', (e) => {
            e.preventDefault();
            document.getElementById('login-form').classList.add('hidden');
            document.getElementById('register-form').classList.remove('hidden');
        });

        document.getElementById('show-login').addEventListener('click', (e) => {
            e.preventDefault();
            document.getElementById('register-form').classList.add('hidden');
            document.getElementById('login-form').classList.remove('hidden');
        });

        // Auth forms
        document.getElementById('form-login').addEventListener('submit', (e) => this.handleLogin(e));
        document.getElementById('form-register').addEventListener('submit', (e) => this.handleRegister(e));

        // Logout
        document.getElementById('btn-logout').addEventListener('click', () => this.logout());

        // Modals
        document.getElementById('btn-add-feed').addEventListener('click', () => this.openModal('modal-add-feed'));
        document.querySelectorAll('.modal-close').forEach(btn => {
            btn.addEventListener('click', (e) => this.closeModal(e.target.closest('.modal').id));
        });

        // Feed form
        document.getElementById('form-add-feed').addEventListener('submit', (e) => this.handleAddFeed(e));

        // Session
        document.getElementById('btn-new-session').addEventListener('click', () => this.openModal('modal-new-session'));
        document.getElementById('btn-welcome-session').addEventListener('click', () => this.openModal('modal-new-session'));
        document.getElementById('form-new-session').addEventListener('submit', (e) => this.handleNewSession(e));

        // Session duration slider
        const slider = document.getElementById('session-duration');
        slider.addEventListener('input', (e) => {
            document.getElementById('duration-value').textContent = e.target.value;
        });

        // Chat
        document.getElementById('btn-close-chat').addEventListener('click', () => this.closeChat());
        document.getElementById('btn-send').addEventListener('click', () => this.sendMessage());
        document.getElementById('message-input').addEventListener('keydown', (e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault();
                this.sendMessage();
            }
        });

        // OPDS Import
        document.getElementById('btn-import-opds').addEventListener('click', () => this.openOPDSModal());
        document.getElementById('btn-opds-import').addEventListener('click', () => this.handleOPDSImport());
    },

    showAuth() {
        document.getElementById('auth-view').classList.remove('hidden');
        document.getElementById('app-view').classList.add('hidden');
    },

    showApp() {
        document.getElementById('auth-view').classList.add('hidden');
        document.getElementById('app-view').classList.remove('hidden');
        this.loadFeeds();
        this.loadSessions();
    },

    async handleLogin(e) {
        e.preventDefault();
        const username = document.getElementById('login-username').value;
        const password = document.getElementById('login-password').value;

        try {
            const data = await API.login(username, password);
            localStorage.setItem('mnl_token', data.token);
            localStorage.setItem('mnl_user_id', data.user_id);
            this.currentUser = { id: data.user_id, token: data.token };
            this.showApp();
        } catch (error) {
            alert('Login failed: ' + error.message);
        }
    },

    async handleRegister(e) {
        e.preventDefault();
        const username = document.getElementById('reg-username').value;
        const displayName = document.getElementById('reg-display').value;
        const password = document.getElementById('reg-password').value;

        try {
            const data = await API.register(username, displayName, password);
            localStorage.setItem('mnl_token', data.token);
            localStorage.setItem('mnl_user_id', data.user_id);
            this.currentUser = { id: data.user_id, token: data.token };
            this.showApp();
        } catch (error) {
            alert('Registration failed: ' + error.message);
        }
    },

    logout() {
        localStorage.removeItem('mnl_token');
        localStorage.removeItem('mnl_user_id');
        this.currentUser = null;
        this.chatManager.disconnect();
        this.showAuth();
    },

    async loadFeeds() {
        try {
            const feeds = await API.getFeeds(this.currentUser.id);
            this.renderFeeds(feeds);
        } catch (error) {
            console.error('Failed to load feeds:', error);
        }
    },

    renderFeeds(feeds) {
        const container = document.getElementById('feed-list');
        if (!feeds || feeds.length === 0) {
            container.innerHTML = '<p class="empty-state">No feeds yet</p>';
            return;
        }

        container.innerHTML = feeds.map(feed => `
            <div class="feed-item">
                <div class="feed-content">
                    <div class="feed-title">${feed.title || 'Untitled Feed'}</div>
                    <div class="feed-url">${feed.url}</div>
                </div>
                <button class="btn-icon btn-refresh" data-feed-id="${feed.id}" title="Refresh feed">
                    🔄
                </button>
            </div>
        `).join('');

        // Add click handlers for refresh buttons
        document.querySelectorAll('.btn-refresh').forEach(btn => {
            btn.addEventListener('click', (e) => {
                e.stopPropagation();
                this.handleRefreshFeed(parseInt(btn.dataset.feedId), btn);
            });
        });
    },

    async handleRefreshFeed(feedId, button) {
        button.disabled = true;
        button.textContent = '⏳';

        try {
            await API.triggerFetch(feedId);
            // Show success feedback
            button.textContent = '✓';
            setTimeout(() => {
                button.textContent = '🔄';
                button.disabled = false;
            }, 2000);
        } catch (error) {
            console.error('Failed to refresh feed:', error);
            button.textContent = '✗';
            setTimeout(() => {
                button.textContent = '🔄';
                button.disabled = false;
            }, 2000);
        }
    },

    async loadSessions() {
        try {
            const sessions = await API.getSessions(this.currentUser.id);
            this.renderSessions(sessions);
        } catch (error) {
            console.error('Failed to load sessions:', error);
        }
    },

    renderSessions(sessions) {
        const container = document.getElementById('session-list');
        if (!sessions || sessions.length === 0) {
            container.innerHTML = '<p class="empty-state">No sessions</p>';
            return;
        }

        container.innerHTML = sessions.map(session => `
            <div class="session-item" data-session-id="${session.id}">
                <div class="feed-title">Session #${session.id}</div>
                <div class="feed-url">${new Date(session.start_at * 1000).toLocaleString()}</div>
            </div>
        `).join('');

        // Add click handlers
        document.querySelectorAll('.session-item').forEach(item => {
            item.addEventListener('click', () => {
                const sessionId = parseInt(item.dataset.sessionId);
                this.openSession(sessionId);
            });
        });
    },

    async handleAddFeed(e) {
        e.preventDefault();
        const url = document.getElementById('feed-url').value;
        const title = document.getElementById('feed-title').value;

        try {
            await API.createFeed(url, title, this.currentUser.id);
            this.closeModal('modal-add-feed');
            document.getElementById('form-add-feed').reset();
            this.loadFeeds();
        } catch (error) {
            alert('Failed to add feed: ' + error.message);
        }
    },

    async handleNewSession(e) {
        e.preventDefault();
        const duration = parseInt(document.getElementById('session-duration').value);
        const durationSeconds = duration * 60;

        try {
            const data = await API.createSession(this.currentUser.id, durationSeconds);
            this.currentSession = data;
            this.closeModal('modal-new-session');
            this.openChat(data.id, durationSeconds);
        } catch (error) {
            alert('Failed to start session: ' + error.message);
        }
    },

    async openChat(sessionId, durationSeconds = null) { // Added durationSeconds parameter with default null
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
            document.getElementById('welcome-screen').classList.add('hidden');
            document.getElementById('chat-screen').classList.remove('hidden');
            document.getElementById('chat-session-id').textContent = sessionId;

            // Load history
            document.getElementById('chat-messages').innerHTML = '';
            if (data.messages && data.messages.length > 0) {
                data.messages.forEach(msg => this.addMessage(msg.author, msg.message));
            }

            // Connect WebSocket
            this.chatManager.connect(sessionId);
            this.chatManager.onMessage = (data) => this.handleChatMessage(data);
            this.chatManager.onStatus = (status) => this.updateChatStatus(status);

        } catch (error) {
            alert('Failed to open session: ' + error.message);
        }
    },

    closeChat() {
        this.chatManager.disconnect();
        this.currentSession = null;
        document.getElementById('chat-screen').classList.add('hidden');
        document.getElementById('welcome-screen').classList.remove('hidden');
    },

    sendMessage() {
        const input = document.getElementById('message-input');
        const message = input.value.trim();

        if (!message) return;

        this.addMessage('user', message);
        this.chatManager.send(message);
        input.value = '';
    },

    handleChatMessage(data) {
        // Handle different message types from WebSocket
        if (data.type === 'progress') {
            // Show progress indicator with status message
            this.updateProgress(data.message || 'Processing...');
        } else if (data.type === 'message' && data.content) {
            // Hide progress and show new message from server
            this.hideProgress();
            this.addMessage('assistant', data.content, data.sources || null);
        } else if (data.type === 'history') {
            // Chat history replay
            this.addMessage(data.role === 'user' ? 'user' : 'assistant', data.content);
        } else if (data.type === 'message' && data.author === 'assistant' && data.message) {
            // Legacy format support
            this.hideProgress();
            this.addMessage('assistant', data.message);
        }
    },

    updateProgress(message) {
        const indicator = document.getElementById('progress-indicator');
        const details = indicator.querySelector('.progress-details');
        indicator.classList.remove('hidden');
        details.textContent = message;
    },

    hideProgress() {
        const indicator = document.getElementById('progress-indicator');
        indicator.classList.add('hidden');
    },

    addMessage(author, text, sources = null) {
        const container = document.getElementById('chat-messages');
        const messageDiv = document.createElement('div');
        messageDiv.className = `message ${author}`;

        const avatar = author === 'user' ? 'U' : 'A';

        // Render Markdown for assistant, escape HTML for user
        let content;
        let usedSources = new Set();

        if (author === 'assistant') {
            // Process inline sources BEFORE markdown parsing
            // Look for [Source Name] patterns
            let processedText = text;

            if (sources && sources.length > 0) {
                // Create a map for quick lookup
                const sourceMap = new Map();
                sources.forEach(s => {
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
        let sourcesHtml = '';
        if (sources && sources.length > 0) {
            // Filter out sources that were already rendered inline
            const remainingSources = sources.filter(s => !usedSources.has(s));

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
        container.scrollTop = container.scrollHeight;
    },

    renderSourceItem(s, inline = false) {
        const domain = this.extractDomain(s.url);
        const faviconUrl = `${domain}/favicon.ico`;
        const sourceName = s.source || 'Unknown';

        const inlineClass = inline ? 'source-inline' : '';
        const style = inline ? 'display: inline-flex; vertical-align: middle; margin: 0 2px;' : '';

        return `
            <a href="${s.url}" target="_blank" rel="noopener noreferrer" class="source-item ${inlineClass}" style="${style}">
                <img src="${faviconUrl}" 
                        alt="" 
                        class="source-icon" 
                        onerror="this.style.display='none'; this.nextElementSibling.style.display='inline';">
                <span class="source-icon-fallback" style="display:none;">📰</span>
                <span class="source-info">
                    <span class="source-name" style="font-weight:600;">${sourceName}</span>
                    ${!inline ? `<span class="source-sep" style="opacity:0.5;">•</span><span class="source-title">${this.truncate(s.title, 40)}</span>` : ''}
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
                    ${sources.map(s => this.renderSourceItem(s)).join('')}
                </div>
            </div>
        `;
    },

    extractDomain(url) {
        try {
            const urlObj = new URL(url);
            return `${urlObj.protocol}//${urlObj.hostname}`;
        } catch {
            return '';
        }
    },

    truncate(text, maxLength) {
        return text.length > maxLength
            ? text.substring(0, maxLength) + '...'
            : text;
    },

    updateChatStatus(status) {
        const statusEl = document.getElementById('chat-status');
        statusEl.textContent = status === 'connected' ? 'Connected' : 'Disconnected';
        statusEl.style.color = status === 'connected' ? 'var(--success)' : 'var(--error)';
    },

    openModal(modalId) {
        document.getElementById(modalId).classList.remove('hidden');
    },

    closeModal(modalId) {
        document.getElementById(modalId).classList.add('hidden');
    },

    escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    },

    openOPDSModal() {
        // Reset modal state
        document.getElementById('opds-file-input').value = '';
        document.getElementById('opds-progress').classList.add('hidden');
        document.getElementById('opds-results').classList.add('hidden');
        document.getElementById('opds-modal').classList.remove('hidden');
    },

    async handleOPDSImport() {
        const fileInput = document.getElementById('opds-file-input');
        const file = fileInput.files[0];

        if (!file) {
            alert('Please select an OPDS file');
            return;
        }

        // Show progress indicator
        document.getElementById('opds-progress').classList.remove('hidden');
        document.getElementById('opds-results').classList.add('hidden');
        document.getElementById('opds-progress-details').textContent = `Uploading ${file.name}...`;

        try {
            const formData = new FormData();
            formData.append('file', file);

            const response = await fetch(`/api/v1/feeds/import/opml?user_id=${this.currentUser.id}`, {
                method: 'POST',
                body: file, // Send raw file data
            });

            if (!response.ok) {
                throw new Error(`Import failed: ${response.statusText}`);
            }

            const result = await response.json();

            // Hide progress, show results
            document.getElementById('opds-progress').classList.add('hidden');
            document.getElementById('opds-results').classList.remove('hidden');

            // Update stats
            document.getElementById('opds-stats-added').textContent = result.added;
            document.getElementById('opds-stats-duplicates').textContent = result.duplicates;
            document.getElementById('opds-stats-errors').textContent = result.errors.length;

            // Show errors if any
            const errorList = document.getElementById('opds-error-list');
            if (result.errors.length > 0) {
                errorList.classList.remove('hidden');
                errorList.innerHTML = '<h4>Errors:</h4><ul>' +
                    result.errors.map(e => `<li>${this.escapeHtml(e)}</li>`).join('') +
                    '</ul>';
            } else {
                errorList.classList.add('hidden');
            }

            // Refresh feed list if any feeds were added
            if (result.added > 0) {
                setTimeout(() => {
                    this.loadFeeds();
                }, 1000);
            }

        } catch (error) {
            console.error('OPDS import error:', error);
            document.getElementById('opds-progress').classList.add('hidden');
            alert(`Import failed: ${error.message}`);
        }
    }
};

// Initialize app when DOM is ready
document.addEventListener('DOMContentLoaded', () => App.init());
