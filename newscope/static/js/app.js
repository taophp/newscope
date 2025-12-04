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
            this.addMessage('assistant', data.content);
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

    addMessage(author, text) {
        const container = document.getElementById('chat-messages');
        const messageDiv = document.createElement('div');
        messageDiv.className = `message ${author}`;

        const avatar = author === 'user' ? 'U' : 'A';

        // Render Markdown for assistant, escape HTML for user
        let content;
        if (author === 'assistant' && window.marked) {
            content = marked.parse(text);
        } else {
            content = this.escapeHtml(text);
        }

        messageDiv.innerHTML = `
            <div class="avatar">${avatar}</div>
            <div class="message-content">${content}</div>
        `;

        container.appendChild(messageDiv);
        container.scrollTop = container.scrollHeight;
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
    }
};

// Initialize app when DOM is ready
document.addEventListener('DOMContentLoaded', () => App.init());
