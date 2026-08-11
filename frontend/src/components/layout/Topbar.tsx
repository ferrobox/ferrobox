import { LogOut, Moon, Sun } from "lucide-react";
import { useTheme } from "next-themes";

import { useAuth } from "@/auth/AuthProvider";
import { HealthIndicator } from "@/components/layout/HealthIndicator";
import { Button } from "@/components/ui/button";

export function Topbar() {
  const { resolvedTheme, setTheme } = useTheme();
  const { user, logout } = useAuth();
  const isDark = resolvedTheme === "dark";

  return (
    <header className="flex h-14 shrink-0 items-center justify-between border-b border-border bg-card px-6">
      <HealthIndicator />

      <div className="flex items-center gap-2">
        {user ? (
          <span className="hidden text-sm text-muted-foreground sm:inline">
            {user.username}
          </span>
        ) : null}
        <Button
          variant="ghost"
          size="icon"
          aria-label={isDark ? "Cambiar a tema claro" : "Cambiar a tema oscuro"}
          onClick={() => setTheme(isDark ? "light" : "dark")}
        >
          {isDark ? <Sun className="size-4" /> : <Moon className="size-4" />}
        </Button>
        <Button variant="ghost" size="icon" aria-label="Cerrar sesión" onClick={logout}>
          <LogOut className="size-4" />
        </Button>
      </div>
    </header>
  );
}
