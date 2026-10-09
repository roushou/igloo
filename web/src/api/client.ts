import createClient, { type Middleware } from "openapi-fetch";
import type { components, paths } from "./schema.gen";
import { TokenStore } from "./token";

export type Repo = components["schemas"]["RepoResource"];

/** How a token fared when checked against the server. */
export type TokenCheck = "accepted" | "rejected" | "unreachable";

/**
 * The only module that talks to the server: every request goes through it, carries the stored
 * bearer token, and a 401 on any of them clears the token and calls the unauthorized handler.
 * It touches the browser only when a request is made, so the build can import it.
 */
export class ApiClient {
  readonly tokens: TokenStore;
  private onUnauthorized: () => void = () => {};
  private client: ReturnType<typeof createClient<paths>> | null = null;

  constructor(tokens = new TokenStore()) {
    this.tokens = tokens;
  }

  /** Sets what happens after a 401, such as returning to the sign-in page. */
  handleUnauthorized(handler: () => void): void {
    this.onUnauthorized = handler;
  }

  /** The repositories, oldest first. */
  async repos(): Promise<Repo[]> {
    const { data, error } = await this.http.GET("/v1/repos");
    if (error || !data) {
      throw new Error("could not list repositories");
    }
    return data;
  }

  /** Checks `token` with `GET /v1/repos` without storing it or triggering the 401 handler. */
  async check(token: string): Promise<TokenCheck> {
    try {
      const response = await fetch(new URL("/v1/repos", window.location.origin), {
        headers: { Authorization: `Bearer ${token}` },
      });
      if (response.ok) return "accepted";
      return response.status === 401 || response.status === 403 ? "rejected" : "unreachable";
    } catch {
      return "unreachable";
    }
  }

  private get http() {
    if (!this.client) {
      const auth: Middleware = {
        onRequest: ({ request }) => {
          const token = this.tokens.get();
          if (token && !request.headers.has("Authorization")) {
            request.headers.set("Authorization", `Bearer ${token}`);
          }
          return request;
        },
        onResponse: ({ response }) => {
          if (response.status === 401) {
            this.tokens.clear();
            this.onUnauthorized();
          }
          return response;
        },
      };
      this.client = createClient<paths>({
        baseUrl: window.location.origin,
        fetch: (request) => globalThis.fetch(request),
      });
      this.client.use(auth);
    }
    return this.client;
  }
}

export const api = new ApiClient();
