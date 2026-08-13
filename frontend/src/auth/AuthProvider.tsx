import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import * as api from "@/api/client";
import type { UserResponse } from "@/api/generated/UserResponse";

interface AuthSession {
  readonly token: string;
  readonly user: UserResponse;
}

interface AuthContextValue {
  readonly user: UserResponse | null;
  readonly token: string | null;
  readonly isLoading: boolean;
  readonly login: (username: string, password: string) => Promise<void>;
  readonly logout: () => void;
  readonly updateCurrentUser: (user: UserResponse) => void;
}

const AuthContext = createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<AuthSession | null>(null);
  const [token, setToken] = useState<string | null>(() => api.getStoredToken());
  const [isLoading, setIsLoading] = useState(() => api.getStoredToken() !== null);

  useEffect(() => {
    const storedToken = api.getStoredToken();
    if (!storedToken) {
      return;
    }

    let cancelled = false;

    void api
      .getMe()
      .then((user) => {
        if (!cancelled) {
          setSession({ token: storedToken, user });
          setToken(storedToken);
        }
      })
      .catch(() => {
        if (!cancelled) {
          api.setStoredToken(null);
          setToken(null);
          setSession(null);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setIsLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  const login = useCallback(async (username: string, password: string) => {
    const result = await api.login({ username, password });
    api.setStoredToken(result.token);
    setSession({ token: result.token, user: result.user });
    setToken(result.token);
    setIsLoading(false);
  }, []);

  const logout = useCallback(() => {
    api.setStoredToken(null);
    setSession(null);
    setToken(null);
    setIsLoading(false);
  }, []);

  const updateCurrentUser = useCallback((user: UserResponse) => {
    setSession((current) => (current ? { ...current, user } : current));
  }, []);

  const value = useMemo<AuthContextValue>(
    () => ({
      user: session?.user ?? null,
      token,
      isLoading,
      login,
      logout,
      updateCurrentUser,
    }),
    [session, token, isLoading, login, logout, updateCurrentUser],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthContextValue {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error("useAuth must be used within AuthProvider");
  }
  return context;
}
