import { Moon, Sun } from "lucide-react";
import { useTheme } from "next-themes";

import { Button } from "@/components/ui/button";
import { HealthIndicator } from "@/components/layout/HealthIndicator";

export function Topbar() {
  const { resolvedTheme, setTheme } = useTheme();
  const isDark = resolvedTheme === "dark";

  return (
    <header className="flex h-14 shrink-0 items-center justify-between border-b border-border bg-card px-6">
      <HealthIndicator />

      <Button
        variant="ghost"
        size="icon"
        aria-label={isDark ? "Cambiar a tema claro" : "Cambiar a tema oscuro"}
        onClick={() => setTheme(isDark ? "light" : "dark")}
      >
        {isDark ? <Sun className="size-4" /> : <Moon className="size-4" />}
      </Button>
    </header>
  );
}
