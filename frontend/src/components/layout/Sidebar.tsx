import { Boxes, Package, Settings2, ShieldCheck, Users } from "lucide-react";
import { NavLink } from "react-router-dom";

import { cn } from "@/lib/utils";

interface NavItem {
  readonly label: string;
  readonly to: string;
  readonly icon: typeof Boxes;
  readonly disabled?: boolean;
}

const NAV_ITEMS: readonly NavItem[] = [
  { label: "Repositorios", to: "/repositories", icon: Boxes },
  { label: "Usuarios", to: "/users", icon: Users, disabled: true },
  { label: "Seguridad", to: "/security", icon: ShieldCheck, disabled: true },
  { label: "Configuración", to: "/settings", icon: Settings2, disabled: true },
];

export function Sidebar() {
  return (
    <aside className="bg-sidebar text-sidebar-foreground flex h-full w-64 flex-col border-r border-sidebar-border">
      <div className="flex h-16 items-center gap-2.5 border-b border-sidebar-border px-5">
        <span className="flex size-8 items-center justify-center rounded-md bg-sidebar-primary text-sidebar-primary-foreground">
          <Package className="size-4.5" strokeWidth={2.25} />
        </span>
        <div className="leading-tight">
          <p className="text-sm font-semibold tracking-wide">FerroBox</p>
          <p className="text-[11px] text-sidebar-foreground/60">Gestor de artefactos</p>
        </div>
      </div>

      <nav className="flex-1 space-y-1 px-3 py-4">
        {NAV_ITEMS.map((item) => (
          <SidebarLink key={item.to} item={item} />
        ))}
      </nav>

      <div className="border-t border-sidebar-border px-5 py-3 text-[11px] text-sidebar-foreground/50">
        FerroBox v0.1.0
      </div>
    </aside>
  );
}

function SidebarLink({ item }: { item: NavItem }) {
  const Icon = item.icon;

  if (item.disabled) {
    return (
      <div
        className="flex cursor-not-allowed items-center justify-between rounded-md px-3 py-2 text-sm text-sidebar-foreground/35"
        title="Próximamente"
      >
        <span className="flex items-center gap-2.5">
          <Icon className="size-4" />
          {item.label}
        </span>
        <span className="rounded-full bg-sidebar-accent/60 px-1.5 py-0.5 text-[10px] font-medium tracking-wide uppercase">
          Pronto
        </span>
      </div>
    );
  }

  return (
    <NavLink
      to={item.to}
      className={({ isActive }) =>
        cn(
          "flex items-center gap-2.5 rounded-md px-3 py-2 text-sm font-medium transition-colors",
          isActive
            ? "bg-sidebar-accent text-sidebar-accent-foreground"
            : "text-sidebar-foreground/75 hover:bg-sidebar-accent/50 hover:text-sidebar-accent-foreground",
        )
      }
    >
      <Icon className="size-4" />
      {item.label}
    </NavLink>
  );
}
