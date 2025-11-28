// WebSocket Chat Manager

class ChatManager {
    constructor() {
        this.ws = null;
        this.sessionId = null;
        this.onMessage = null;
        this.onStatus = null;
        this.timerInterval = null;
        this.sessionStartTime = null;
        this.sessionDuration = 0; // in seconds
        this.isLoading = false;
    }

    setSessionDuration(durationSeconds) {
        this.sessionDuration = durationSeconds;
    }

    showLoading() {
        this.isLoading = true;
        const loadingEl = document.getElementById('chat-loading');
        const messagesEl = document.getElementById('chat-messages');
        if (loadingEl) loadingEl.classList.remove('hidden');
        if (messagesEl) messagesEl.classList.add('hidden');
    }

    hideLoading() {
        this.isLoading = false;
        const loadingEl = document.getElementById('chat-loading');
        const messagesEl = document.getElementById('chat-messages');
        if (loadingEl) loadingEl.classList.add('hidden');
        if (messagesEl) messagesEl.classList.remove('hidden');
    }

    startTimer() {
        this.sessionStartTime = Date.now();
        this.updateTimer();

        // Update timer every second
        this.timerInterval = setInterval(() => {
            this.updateTimer();
        }, 1000);
    }

    updateTimer() {
        if (!this.sessionStartTime) return;

        const elapsed = Math.floor((Date.now() - this.sessionStartTime) / 1000);
        const remaining = Math.max(0, this.sessionDuration - elapsed);
        const percentUsed = this.sessionDuration > 0 ? (elapsed / this.sessionDuration) * 100 : 0;

        const timerEl = document.getElementById('session-timer');
        if (!timerEl) return;

        // Format time as MM:SS
        const formatTime = (seconds) => {
            const mins = Math.floor(seconds / 60);
            const secs = seconds % 60;
            return `${mins}:${secs.toString().padStart(2, '0')}`;
        };

        timerEl.textContent = `${formatTime(elapsed)} / ${formatTime(this.sessionDuration)}`;

        // Update color based on time used
        timerEl.className = 'session-timer';
        if (percentUsed >= 100) {
            timerEl.classList.add('expired');
        } else if (percentUsed >= 80) {
            timerEl.classList.add('warning');
        }
    }

    stopTimer() {
        if (this.timerInterval) {
            clearInterval(this.timerInterval);
            this.timerInterval = null;
        }
    }

    connect(sessionId) {
        this.sessionId = sessionId;
        const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const url = `${protocol}//${window.location.host}/ws/chat?session_id=${sessionId}`;

        this.ws = new WebSocket(url);

        this.ws.onopen = () => {
            console.log('WebSocket connected');
            this.showLoading();
            this.startTimer();
            if (this.onStatus) this.onStatus('connected');
        };

        this.ws.onmessage = (event) => {
            try {
                const data = JSON.parse(event.data);
                console.log('Message received:', data);

                // Hide loading on first message (press review)
                if (this.isLoading) {
                    this.hideLoading();
                }

                if (this.onMessage) this.onMessage(data);
            } catch (error) {
                console.error('Failed to parse message:', error);
            }
        };

        this.ws.onerror = (error) => {
            console.error('WebSocket error:', error);
            this.hideLoading();
            if (this.onStatus) this.onStatus('error');
        };

        this.ws.onclose = () => {
            console.log('WebSocket disconnected');
            this.stopTimer();
            if (this.onStatus) this.onStatus('disconnected');
        };
    }

    send(message) {
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
            this.ws.send(JSON.stringify({ message }));
        } else {
            console.error('WebSocket not connected');
        }
    }

    disconnect() {
        if (this.ws) {
            this.ws.close();
            this.ws = null;
        }
        this.stopTimer();
    }
}

window.ChatManager = ChatManager;
