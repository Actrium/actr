import { useState } from "react";
import { api } from "../lib/api";

const CONFIG_TYPES = [
  { value: 1, label: "TURN Realm" },
  { value: 2, label: "STUN Bind Address" },
  { value: 3, label: "Log Level" },
  { value: 4, label: "Service Enabled" },
  { value: 99, label: "Custom" },
];

export function Settings() {
  const [configType, setConfigType] = useState(1);
  const [configKey, setConfigKey] = useState("");
  const [configValue, setConfigValue] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  async function handleLookup() {
    if (!configKey) return;
    setLoading(true);
    setError("");
    try {
      const data = await api.getConfig(configType, configKey);
      setConfigValue(data.config_value);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load config");
      setConfigValue(null);
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-xl font-semibold text-gray-900">Settings</h1>
        <p className="text-sm text-gray-500 mt-1">View configuration values</p>
      </div>

      <div className="rounded-xl border border-gray-200 bg-white p-5">
        <div className="space-y-4">
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Config Type
              </label>
              <select
                value={configType}
                onChange={(e) => setConfigType(Number(e.target.value))}
                className="w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
              >
                {CONFIG_TYPES.map((ct) => (
                  <option key={ct.value} value={ct.value}>
                    {ct.label}
                  </option>
                ))}
              </select>
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Config Key
              </label>
              <input
                type="text"
                value={configKey}
                onChange={(e) => setConfigKey(e.target.value)}
                className="w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
                placeholder="e.g. default"
              />
            </div>
            <div className="flex items-end">
              <button
                onClick={handleLookup}
                disabled={loading || !configKey}
                className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50 transition-colors"
              >
                {loading ? "Loading..." : "Lookup"}
              </button>
            </div>
          </div>

          {error && (
            <p className="text-sm text-red-600">{error}</p>
          )}

          {configValue !== null && (
            <div className="rounded-lg bg-gray-50 p-4">
              <p className="text-xs font-medium text-gray-500 mb-1">Value</p>
              <pre className="text-sm font-mono text-gray-900 whitespace-pre-wrap break-all">
                {configValue}
              </pre>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
