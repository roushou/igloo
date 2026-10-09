/** The bearer token, kept in `localStorage` for the browser profile. */
export class TokenStore {
  static readonly KEY = "igloo.token";

  get(): string | null {
    return localStorage.getItem(TokenStore.KEY);
  }

  set(token: string): void {
    localStorage.setItem(TokenStore.KEY, token);
  }

  clear(): void {
    localStorage.removeItem(TokenStore.KEY);
  }
}
