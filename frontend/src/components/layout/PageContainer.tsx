import type { ReactNode } from "react";
import { Outlet } from "react-router-dom";

export function PageContainer({ children }: { children?: ReactNode }) {
  return (
    <div className="mx-auto w-full max-w-6xl flex-1 overflow-y-auto px-8 py-8">
      {children ?? <Outlet />}
    </div>
  );
}
