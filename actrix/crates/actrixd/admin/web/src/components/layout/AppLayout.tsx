import { Outlet } from "react-router-dom";
import { Sidebar } from "./Sidebar";

export function AppLayout() {
  return (
    <div className="flex min-h-screen items-stretch bg-gray-50">
      <Sidebar />
      <main className="min-w-0 flex-1 overflow-x-hidden px-4 py-5 lg:px-5 lg:py-6">
        <Outlet />
      </main>
    </div>
  );
}
