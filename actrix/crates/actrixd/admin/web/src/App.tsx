import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { AuthProvider } from "./lib/auth";
import { AppLayout } from "./components/layout/AppLayout";
import { Login } from "./pages/Login";
import { Dashboard } from "./pages/Dashboard";
import { Realms } from "./pages/Realms";
import { Settings } from "./pages/Settings";
import { ConfigEditor } from "./pages/ConfigEditor";
import { StunService } from "./pages/services/StunService";
import { TurnService } from "./pages/services/TurnService";
import { SignalingService } from "./pages/services/SignalingService";
import { AisService } from "./pages/services/AisService";
import { KsService } from "./pages/services/KsService";
import { Overview2Page } from "./pages/general/Overview2Page";
import { RecordingSection } from "./pages/platform/RecordingSection";
import { NetworkPage } from "./pages/host/NetworkPage";
import { StatusPage } from "./pages/host/StatusPage";
import { AboutPage } from "./pages/About";
import type { ReactNode } from "react";

function RequireAuth({ children }: { children: ReactNode }) {
  // TODO: Re-enable authentication check
  // const { isAuthenticated } = useAuth();
  // if (!isAuthenticated) return <Navigate to="/admin/login" replace />;
  return <>{children}</>;
}

export function App() {
  return (
    <BrowserRouter>
      <AuthProvider>
        <Routes>
          <Route path="/admin/login" element={<Login />} />
          <Route
            path="/admin"
            element={
              <RequireAuth>
                <AppLayout />
              </RequireAuth>
            }
          >
            <Route index element={<Dashboard />} />
            <Route path="realms" element={<Realms />} />
            <Route path="config" element={<ConfigEditor />} />
            <Route path="settings" element={<Settings />} />
            <Route path="general/architecture" element={<Overview2Page />} />
            <Route path="general/recording" element={<RecordingSection />} />
            <Route path="about" element={<AboutPage />} />
            <Route path="host/network" element={<NetworkPage />} />
            <Route path="host/status" element={<StatusPage />} />
            <Route path="services/stun" element={<StunService />} />
            <Route path="services/turn" element={<TurnService />} />
            <Route path="services/signaling" element={<SignalingService />} />
            <Route path="services/ais" element={<AisService />} />
            <Route path="services/ks" element={<KsService />} />
          </Route>
          <Route path="*" element={<Navigate to="/admin" replace />} />
        </Routes>
      </AuthProvider>
    </BrowserRouter>
  );
}
