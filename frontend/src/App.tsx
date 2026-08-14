import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactQueryDevtools } from "@tanstack/react-query-devtools";
import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";

import { AuthProvider } from "@/auth/AuthProvider";
import { RequireAuth } from "@/auth/RequireAuth";
import { AppShell } from "@/components/layout/AppShell";
import { PageContainer } from "@/components/layout/PageContainer";
import { ThemeProvider } from "@/components/theme-provider";
import { Toaster } from "@/components/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { LoginPage } from "@/pages/LoginPage";
import NotFoundPage from "@/pages/NotFoundPage";
import { RepositoriesLayout } from "@/pages/RepositoriesLayout";
import { RepositoriesPage } from "@/pages/RepositoriesPage";
import { RepositoryDetailPage } from "@/pages/RepositoryDetailPage";
import { SecurityPage } from "@/pages/SecurityPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { UsersPage } from "@/pages/UsersPage";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 10_000,
      refetchOnWindowFocus: false,
    },
  },
});

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <TooltipProvider>
          <AuthProvider>
            <BrowserRouter>
              <Routes>
                <Route path="/login" element={<LoginPage />} />
                <Route element={<RequireAuth />}>
                  <Route element={<AppShell />}>
                    <Route index element={<Navigate to="/repositories" replace />} />
                    <Route path="repositories" element={<RepositoriesLayout />}>
                      <Route index element={<RepositoriesPage />} />
                      <Route path=":repositoryId" element={<RepositoryDetailPage />} />
                    </Route>
                    <Route element={<PageContainer />}>
                      <Route path="security" element={<SecurityPage />} />
                      <Route path="users" element={<UsersPage />} />
                      <Route path="settings" element={<SettingsPage />} />
                      <Route path="*" element={<NotFoundPage />} />
                    </Route>
                  </Route>
                </Route>
              </Routes>
            </BrowserRouter>
          </AuthProvider>
          <Toaster richColors closeButton position="top-right" />
        </TooltipProvider>
      </ThemeProvider>
      {import.meta.env.DEV ? <ReactQueryDevtools initialIsOpen={false} /> : null}
    </QueryClientProvider>
  );
}
