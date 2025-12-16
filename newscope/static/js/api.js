// API Client for MyNewsLens

const API = {
  baseURL: window.location.origin,

  async request(path, options = {}) {
    const url = `${this.baseURL}${path}`;
    const token = localStorage.getItem("mnl_token");

    const headers = {
      "Content-Type": "application/json",
      ...options.headers,
    };

    if (token) {
      headers["Authorization"] = `Bearer ${token}`;
    }

    try {
      const response = await fetch(url, {
        ...options,
        headers,
      });

      if (!response.ok) {
        throw new Error(`HTTP ${response.status}: ${response.statusText}`);
      }

      return await response.json();
    } catch (error) {
      console.error("API Error:", error);
      throw error;
    }
  },

  // Auth
  async register(username, displayName, password) {
    return this.request("/api/v1/register", {
      method: "POST",
      body: JSON.stringify({ username, display_name: displayName, password }),
    });
  },

  async login(username, password) {
    return this.request("/api/v1/login", {
      method: "POST",
      body: JSON.stringify({ username, password }),
    });
  },

  // Soft logout / token revocation: call server to revoke the current JWT.
  // The server endpoint expects a JSON body: { token: "<jwt>" }.
  // On success the client removes stored auth state locally.
  async logout() {
    const token = localStorage.getItem("mnl_token");
    if (!token) {
      // nothing to revoke; resolve to a lightweight object for callers
      return { ok: false, message: "no token" };
    }

    // Send revoke request to server; this.request will throw on non-2xx
    const result = await this.request("/api/v1/logout", {
      method: "POST",
      body: JSON.stringify({ token }),
    });

    // If successful, clear client-side auth
    localStorage.removeItem("mnl_token");
    localStorage.removeItem("mnl_user_id");

    return result;
  },

  // Feeds
  async getFeeds(userId) {
    return this.request(`/api/v1/feeds?user_id=${userId}`);
  },

  async createFeed(url, title, userId) {
    return this.request("/api/v1/feeds", {
      method: "POST",
      body: JSON.stringify({ url, title, user_id: userId }),
    });
  },

  async triggerFetch(feedId) {
    return this.request("/api/v1/fetch", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ feed_id: feedId }),
    });
  },

  // Sessions
  async createSession(userId, durationSeconds) {
    return this.request("/api/v1/sessions", {
      method: "POST",
      body: JSON.stringify({
        user_id: userId,
        duration_seconds: durationSeconds,
      }),
    });
  },

  async getSessions(userId) {
    return this.request(`/api/v1/sessions?user_id=${userId}`);
  },

  async getSession(sessionId) {
    return this.request(`/api/v1/sessions/${sessionId}`);
  },

  async updateSession(sessionId, title) {
    return this.request(`/api/v1/sessions/${sessionId}`, {
      method: "PUT",
      body: JSON.stringify({ title }),
    });
  },
};

window.API = API;
