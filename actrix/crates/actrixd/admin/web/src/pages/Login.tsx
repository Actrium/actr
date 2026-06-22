import { useEffect, useState, type FormEvent } from "react";
import { useNavigate } from "react-router-dom";
import { useAuth } from "../lib/auth";
import { api } from "../lib/api";

export function Login() {
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [nodeName, setNodeName] = useState("unavailable");
  const { login, isAuthenticated } = useAuth();
  const navigate = useNavigate();

  useEffect(() => {
    if (isAuthenticated) {
      navigate("/admin", { replace: true });
    }
  }, [isAuthenticated, navigate]);

  useEffect(() => {
    let active = true;

    const loadNodeName = async () => {
      try {
        const data = await api.getPublicNodeName();
        const value = data.name.trim();
        if (!active) {
          return;
        }
        if (value.length > 0) {
          setNodeName(value);
        } else {
          setNodeName("unavailable");
        }
      } catch {
        if (active) {
          setNodeName("unavailable");
        }
      }
    };

    void loadNodeName();
    return () => {
      active = false;
    };
  }, []);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setError("");
    setLoading(true);
    try {
      await login(password);
      navigate("/admin", { replace: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : "Login failed");
    } finally {
      setLoading(false);
    }
  }

  if (isAuthenticated) {
    return null;
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-gray-50">
      <div className="w-full max-w-md">
        <div className="rounded-xl border border-gray-200 bg-white p-8 shadow-sm">
          <div className="mb-6">
            <div className="flex items-baseline justify-between gap-3">
              <h1 className="inline-flex min-w-0 items-baseline gap-2">
                <span className="text-3xl font-bold tracking-tight text-gray-900">
                  Actrix
                </span>
                <span className="relative -top-px rounded bg-blue-100 px-2 py-px text-[1.25rem] font-semibold text-blue-600">
                  Admin
                </span>
              </h1>
              <span
                className="ml-auto max-w-40 truncate text-right text-base font-medium text-gray-500"
                title={`@${nodeName}`}
              >
                @{nodeName}
              </span>
            </div>
            <p className="mt-2 text-base text-gray-500">
              Enter your password to continue
            </p>
          </div>

          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label
                htmlFor="password"
                className="block text-sm font-medium text-gray-700 mb-1"
              >
                Password
              </label>
              <input
                id="password"
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                placeholder="Enter admin password"
                required
                autoFocus
              />
            </div>

            {error && (
              <p className="text-sm text-red-600">{error}</p>
            )}

            <button
              type="submit"
              disabled={loading || !password}
              className="w-full rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            >
              {loading ? "Signing in..." : "Sign in"}
            </button>
          </form>
        </div>
      </div>
    </div>
  );
}
