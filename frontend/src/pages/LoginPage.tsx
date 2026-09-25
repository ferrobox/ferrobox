import { type FormEvent, useEffect, useState } from "react";
import { Navigate, useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { Package } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { ApiError } from "@/api/client";
import { useOidcStatus } from "@/api/queries";
import { useAuth } from "@/auth/AuthProvider";
import { LanguageSwitcher } from "@/components/layout/LanguageSwitcher";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function LoginPage() {
  const { token, login, completeSso, isLoading } = useAuth();
  const { t } = useTranslation();
  const navigate = useNavigate();
  const location = useLocation();
  const [searchParams] = useSearchParams();
  const { data: oidc } = useOidcStatus();
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [ssoPending, setSsoPending] = useState(() => window.location.hash.includes("sso_token="));

  const from =
    (location.state as { from?: string } | null)?.from &&
    (location.state as { from?: string }).from !== "/login"
      ? (location.state as { from: string }).from
      : "/repositories";

  useEffect(() => {
    const error = searchParams.get("sso_error");
    if (error) {
      toast.error(error);
    }
  }, [searchParams]);

  useEffect(() => {
    const hash = window.location.hash.startsWith("#")
      ? window.location.hash.slice(1)
      : window.location.hash;
    const tokenFromHash = new URLSearchParams(hash).get("sso_token");
    if (!tokenFromHash) {
      setSsoPending(false);
      return;
    }
    let cancelled = false;
    setSsoPending(true);
    void completeSso(tokenFromHash)
      .then(() => {
        if (!cancelled) {
          window.history.replaceState(null, "", window.location.pathname);
          navigate(from, { replace: true });
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          window.history.replaceState(null, "", window.location.pathname);
          toast.error(error instanceof ApiError ? error.message : t("login.ssoFailed"));
          setSsoPending(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [completeSso, from, navigate, t]);

  if (!isLoading && token) {
    return <Navigate to={from} replace />;
  }

  async function onSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmitting(true);
    try {
      await login(username.trim(), password);
      navigate(from, { replace: true });
    } catch (error) {
      const message = error instanceof ApiError ? error.message : t("login.failed");
      toast.error(message);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="relative flex min-h-screen items-center justify-center overflow-hidden px-4">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(ellipse_at_top,_oklch(0.92_0.04_250)_0%,_transparent_55%),linear-gradient(160deg,_oklch(0.97_0.02_250),_oklch(0.94_0.03_230))]"
      />
      <div
        aria-hidden
        className="pointer-events-none absolute -left-24 top-24 size-72 rounded-full bg-primary/10 blur-3xl motion-safe:animate-pulse"
      />
      <div
        aria-hidden
        className="pointer-events-none absolute -right-16 bottom-16 size-64 rounded-full bg-accent/40 blur-3xl"
      />

      <div className="absolute top-4 right-4">
        <LanguageSwitcher />
      </div>

      <div className="relative w-full max-w-md animate-in fade-in slide-in-from-bottom-2 duration-500">
        <div className="mb-8 text-center">
          <div className="mx-auto mb-4 flex size-12 items-center justify-center rounded-xl bg-primary text-primary-foreground shadow-sm">
            <Package className="size-6" strokeWidth={2.25} />
          </div>
          <h1 className="text-3xl font-semibold tracking-tight text-foreground">
            {t("app.name")}
          </h1>
          <p className="mt-2 text-sm text-muted-foreground">{t("login.subtitle")}</p>
        </div>

        <form
          onSubmit={(event) => void onSubmit(event)}
          className="space-y-5 rounded-xl border border-border/80 bg-card/90 p-6 shadow-sm backdrop-blur"
        >
          <div className="space-y-2">
            <Label htmlFor="username">{t("login.username")}</Label>
            <Input
              id="username"
              autoComplete="username"
              value={username}
              onChange={(event) => setUsername(event.target.value)}
              required
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="password">{t("login.password")}</Label>
            <Input
              id="password"
              type="password"
              autoComplete="current-password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              required
            />
          </div>
          <Button type="submit" className="w-full" disabled={submitting || ssoPending}>
            {submitting ? t("login.submitting") : t("login.submit")}
          </Button>
          {oidc?.enabled ? (
            <a
              href="/api/auth/oidc/start"
              className="inline-flex h-9 w-full items-center justify-center rounded-md border border-border bg-background px-4 text-sm font-medium hover:bg-accent"
            >
              {ssoPending ? t("login.ssoSubmitting") : t("login.sso")}
            </a>
          ) : null}
          <p className="text-center text-xs text-muted-foreground">{t("login.forgot")}</p>
        </form>
      </div>
    </div>
  );
}
