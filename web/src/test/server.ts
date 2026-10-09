import { api } from "@/api/client";

export const TOKEN = "test-token";

/** Signs the test in, as the sign-in page would. */
export function signIn(): void {
  api.tokens.set(TOKEN);
}
