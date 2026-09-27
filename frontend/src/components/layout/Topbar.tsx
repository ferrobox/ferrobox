import { type FormEvent, useState } from "react";
import { LogOut, Moon, Search, Sun } from "lucide-react";
import { useTheme } from "next-themes";
import { useTranslation } from "react-i18next";
import { useLocation, useNavigate, useSearchParams } from "react-router-dom";

import { useAuth } from "@/auth/AuthProvider";
import { roleLabel } from "@/auth/roles";
import { LanguageSwitcher } from "@/components/layout/LanguageSwitcher";
import { StorageUsage } from "@/components/layout/StorageUsage";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export function Topbar() {
  const { resolvedTheme, setTheme } = useTheme();
  const { user, logout } = useAuth();
  const { t } = useTranslation();
  const isDark = resolvedTheme === "dark";
  const location = useLocation();
  const [params] = useSearchParams();
  const urlQuery = location.pathname === "/search" ? (params.get("q") ?? "") : "";

  return (
    <header className="flex h-14 shrink-0 items-center gap-4 border-b border-border bg-card px-6">
      <StorageUsage />

      <TopbarSearch key={urlQuery} initial={urlQuery} />

      <div className="flex items-center gap-2">
        {user ? (
          <span className="hidden items-center gap-2 text-sm text-muted-foreground sm:flex">
            {user.username}
            <Badge variant="outline">{roleLabel(user.role)}</Badge>
          </span>
        ) : null}
        <LanguageSwitcher />
        <Button
          variant="ghost"
          size="icon"
          aria-label={isDark ? t("topbar.themeToLight") : t("topbar.themeToDark")}
          onClick={() => setTheme(isDark ? "light" : "dark")}
        >
          {isDark ? <Sun className="size-4" /> : <Moon className="size-4" />}
        </Button>
        <Button variant="ghost" size="icon" aria-label={t("topbar.logout")} onClick={logout}>
          <LogOut className="size-4" />
        </Button>
      </div>
    </header>
  );
}

function TopbarSearch({ initial }: { initial: string }) {
  const navigate = useNavigate();
  const { t } = useTranslation();
  const [query, setQuery] = useState(initial);

  function onSearch(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = query.trim();
    navigate(trimmed.length > 0 ? `/search?q=${encodeURIComponent(trimmed)}` : "/search");
  }

  return (
    <form onSubmit={onSearch} className="flex min-w-0 flex-1 justify-center">
      <div className="relative w-full max-w-xl">
        <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("topbar.searchPlaceholder")}
          aria-label={t("topbar.searchAria")}
          className="pl-8"
        />
      </div>
    </form>
  );
}
