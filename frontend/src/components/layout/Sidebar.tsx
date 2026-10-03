import { Boxes, Database, FlaskConical, ScrollText, Search, Settings2, ShieldCheck, Users, UsersRound } from "lucide-react";
import { useTranslation } from "react-i18next";
import { NavLink } from "react-router-dom";

import { useAuth } from "@/auth/AuthProvider";
import { canManageUsers } from "@/auth/roles";
import { cn } from "@/lib/utils";

interface NavItem {
  readonly labelKey: string;
  readonly to: string;
  readonly icon: typeof Boxes;
  readonly disabled?: boolean;
  readonly adminOnly?: boolean;
}

const NAV_ITEMS: readonly NavItem[] = [
  { labelKey: "nav.repositories", to: "/repositories", icon: Boxes },
  { labelKey: "nav.search", to: "/search", icon: Search },
  { labelKey: "nav.assays", to: "/assays", icon: FlaskConical },
  { labelKey: "nav.security", to: "/security", icon: ShieldCheck },
  { labelKey: "nav.feed", to: "/feed", icon: Database, adminOnly: true },
  { labelKey: "nav.users", to: "/users", icon: Users, adminOnly: true },
  { labelKey: "nav.groups", to: "/groups", icon: UsersRound, adminOnly: true },
  { labelKey: "nav.audit", to: "/audit", icon: ScrollText, adminOnly: true },
  { labelKey: "nav.settings", to: "/settings", icon: Settings2 },
];

export function Sidebar() {
  const { user } = useAuth();
  const { t } = useTranslation();
  const items = NAV_ITEMS.filter((item) => !item.adminOnly || canManageUsers(user?.role));

  return (
    <aside className="bg-sidebar text-sidebar-foreground flex h-full w-64 flex-col border-r border-sidebar-border">
      <div className="flex h-16 items-center gap-2.5 border-b border-sidebar-border px-5">
        <img src="/svg_fb_logo_transparency.svg" alt="" width={32} height={37} className="h-8 w-auto shrink-0" />
        <div className="leading-tight">
          <p className="text-sm font-bold tracking-tight">
            <span className="text-brand-ferro">Ferro</span>
            <span className="text-brand-box">Box</span>
          </p>
          <p className="text-[11px] text-sidebar-foreground/60">{t("app.tagline")}</p>
        </div>
      </div>

      <nav className="flex-1 space-y-1 px-3 py-4">
        {items.map((item) => (
          <SidebarLink key={item.to} item={item} />
        ))}
      </nav>

      <div className="border-t border-sidebar-border px-5 py-3 text-[11px] text-sidebar-foreground/50">
        {t("app.version")}
      </div>
    </aside>
  );
}

function SidebarLink({ item }: { item: NavItem }) {
  const Icon = item.icon;
  const { t } = useTranslation();
  const label = t(item.labelKey);

  if (item.disabled) {
    return (
      <div
        className="flex cursor-not-allowed items-center justify-between rounded-md px-3 py-2 text-sm text-sidebar-foreground/35"
        title={t("nav.comingSoon")}
      >
        <span className="flex items-center gap-2.5">
          <Icon className="size-4" />
          {label}
        </span>
        <span className="rounded-full bg-sidebar-accent/60 px-1.5 py-0.5 text-[10px] font-medium tracking-wide uppercase">
          {t("nav.soon")}
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
      {label}
    </NavLink>
  );
}
