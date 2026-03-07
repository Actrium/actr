import { useState, useEffect, useRef } from "react";
import { Link } from "react-router-dom";
import { ExternalLink, X, Copy, Check } from "lucide-react";
import { api, type ResolvedField } from "../../lib/api";

/* ── Enable bitmask calculator ─────────────────────────────── */

const ENABLE_SERVICES = [
  { name: "Signaling", bit: 1 },
  { name: "STUN", bit: 2 },
  { name: "TURN", bit: 4 },
  { name: "AIS", bit: 8 },
  { name: "KS", bit: 16 },
] as const;

function EnableCalculator({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  const mask = parseInt(value, 10) || 0;
  const [copied, setCopied] = useState(false);

  const toggle = (bit: number) => {
    onChange(String(mask ^ bit));
  };

  const handleCopy = async () => {
    await navigator.clipboard.writeText(String(mask));
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="rounded-lg border border-gray-200 bg-gray-50 p-3 space-y-2.5">
      <p className="text-xs font-medium text-gray-500">Enable calculator</p>
      <div className="flex flex-wrap gap-2">
        {ENABLE_SERVICES.map((s) => {
          const on = (mask & s.bit) !== 0;
          return (
            <label
              key={s.name}
              className={`inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs font-medium cursor-pointer select-none transition-colors ${
                on
                  ? "border-green-300 bg-green-50 text-green-700"
                  : "border-gray-200 bg-white text-gray-400"
              }`}
            >
              <input
                type="checkbox"
                checked={on}
                onChange={() => toggle(s.bit)}
                className="sr-only"
              />
              <span
                className={`h-3 w-3 rounded border flex items-center justify-center text-[8px] ${
                  on
                    ? "border-green-500 bg-green-500 text-white"
                    : "border-gray-300 bg-white"
                }`}
              >
                {on && "✓"}
              </span>
              {s.name}
              <span className="text-[10px] opacity-50">({s.bit})</span>
            </label>
          );
        })}
      </div>
      <div className="flex items-center gap-2 pt-0.5">
        <span className="text-xs text-gray-400">Result:</span>
        <span className="font-mono text-sm font-semibold text-gray-900">{mask}</span>
        <button
          type="button"
          onClick={handleCopy}
          className="inline-flex items-center gap-1 rounded border border-gray-200 px-1.5 py-0.5 text-[10px] text-gray-400 hover:text-gray-600 hover:border-gray-300 transition-colors"
        >
          {copied ? (
            <><Check className="h-3 w-3 text-green-500" /> copied</>
          ) : (
            <><Copy className="h-3 w-3" /> copy</>
          )}
        </button>
      </div>
    </div>
  );
}

/* ── ConfigFieldModal ─────────────────────────────────────── */

export function ConfigFieldModal({
  field,
  tomlLine,
  onClose,
  onSaved,
}: {
  field: ResolvedField;
  tomlLine?: number;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [value, setValue] = useState(field.override_value ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [editing, setEditing] = useState(false);
  const editRef = useRef<HTMLTableRowElement>(null);

  // The "pristine" value when editing started — used to detect changes
  const pristine = field.override_value ?? "";
  const dirty = editing && value !== pristine;

  // Escape on the whole modal closes it (unless editing override, then cancel edit first)
  useEffect(() => {
    const handleEsc = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (editing) cancelEdit();
        else onClose();
      }
    };
    window.addEventListener("keydown", handleEsc);
    return () => window.removeEventListener("keydown", handleEsc);
  }, [onClose, editing]);

  // Click outside the override row cancels editing
  useEffect(() => {
    if (!editing) return;
    function handleClick(e: MouseEvent) {
      if (editRef.current && !editRef.current.contains(e.target as Node)) {
        cancelEdit();
      }
    }
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [editing]);

  function cancelEdit() {
    setValue(field.override_value ?? "");
    setEditing(false);
  }

  const handleSet = async () => {
    if (!value.trim()) return;
    setSaving(true);
    setError("");
    try {
      await api.setOverride(field.key, value.trim());
      onSaved();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="bg-white rounded-xl shadow-xl w-full max-w-md mx-4">
        {/* Header */}
        <div className="px-5 py-3.5 border-b border-gray-200">
          <div className="flex items-center justify-between">
            <div className="min-w-0">
              <h3 className="text-sm font-semibold text-gray-900 font-mono truncate">
                {field.key}
              </h3>
              <p className="text-xs text-gray-500 mt-0.5">{field.description}</p>
            </div>
            <button
              onClick={onClose}
              className="ml-3 text-gray-400 hover:text-gray-600 transition-colors flex-shrink-0"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
          <div className="flex items-center gap-1 mt-2">
            <span className="px-1.5 py-0.5 rounded bg-gray-100 text-gray-500 text-[10px]">
              {field.value_type}
            </span>
            <span className={`px-1.5 py-0.5 rounded text-[10px] ${
              field.dynamic ? "bg-green-50 text-green-600" : "bg-gray-50 text-gray-400"
            }`}>
              {field.dynamic ? "dynamic" : "static"}
            </span>
            {field.reloadable && (
              <span className="px-1.5 py-0.5 rounded bg-blue-50 text-blue-600 text-[10px]">
                reloadable
              </span>
            )}
          </div>
          {field.key === "enable" && (
            <div className="mt-3">
              <EnableCalculator value={value || field.effective_value} onChange={setValue} />
            </div>
          )}
        </div>

        {/* Body */}
        <div className="px-5 py-4 space-y-3">
          <table className="w-full text-sm">
            <tbody>
              {/* Effective */}
              <tr className="border-b border-gray-200">
                <td className="py-1.5 pr-3 text-xs text-gray-400 whitespace-nowrap w-24">
                  Effective:
                </td>
                <td className="py-1.5 font-mono font-medium text-gray-900">
                  {field.effective_value}
                  <span className="ml-2 text-xs font-normal text-gray-400">
                    ({field.source === "config_file" ? "config" : field.source})
                  </span>
                </td>
              </tr>

              {/* Override */}
              <tr
                ref={editRef}
                className={`border-b border-gray-100 h-10 ${field.dynamic && !editing ? "cursor-pointer hover:bg-gray-50" : ""}`}
                onClick={field.dynamic && !editing ? () => setEditing(true) : undefined}
              >
                <td className="pr-3 text-xs text-gray-400 whitespace-nowrap w-24 align-middle">
                  Override
                </td>
                <td className="font-mono align-middle">
                  {editing ? (
                    <div className="flex items-center gap-1.5">
                      {field.choices && field.choices.length > 0 ? (
                        <select
                          value={value || field.effective_value}
                          onChange={(e) => setValue(e.target.value)}
                          onKeyDown={(e) => { if (e.key === "Escape") cancelEdit(); }}
                          className="flex-1 h-7 rounded border border-gray-300 bg-white px-2 text-sm font-mono focus:outline-none focus:ring-1 focus:ring-blue-400 focus:border-transparent"
                          disabled={saving}
                          autoFocus
                        >
                          {field.choices.map((c) => (
                            <option key={c} value={c}>{c}</option>
                          ))}
                        </select>
                      ) : (
                        <input
                          type="text"
                          value={value}
                          onChange={(e) => setValue(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Escape") cancelEdit();
                            else if (e.key === "Enter" && value.trim()) handleSet();
                          }}
                          placeholder={field.override_value ?? field.effective_value}
                          className="flex-1 h-7 rounded border border-gray-300 bg-white px-2 text-sm font-mono focus:outline-none focus:ring-1 focus:ring-blue-400 focus:border-transparent"
                          disabled={saving}
                          autoFocus
                        />
                      )}
                      {dirty && (
                        <button
                          onMouseDown={(e) => e.preventDefault()}
                          onClick={handleSet}
                          disabled={saving || !value.trim()}
                          className="h-7 rounded border border-blue-600 bg-blue-600 px-2.5 text-xs font-medium text-white hover:bg-blue-700 disabled:opacity-50 transition-colors"
                        >
                          Set
                        </button>
                      )}
                      <button
                        onMouseDown={(e) => e.preventDefault()}
                        onClick={cancelEdit}
                        className="h-7 rounded border border-gray-300 px-2.5 text-xs font-medium text-gray-500 hover:bg-gray-100 transition-colors"
                      >
                        Cancel
                      </button>
                    </div>
                  ) : field.dynamic ? (
                    field.override_value != null ? (
                      <span className="text-amber-600">{field.override_value}</span>
                    ) : (
                      <span className="text-gray-300 italic">click to set</span>
                    )
                  ) : (
                    <span className="text-gray-300 italic">static</span>
                  )}
                </td>
              </tr>

              {/* Config file */}
              <tr className="border-b border-gray-100">
                <td className="py-1.5 pr-3 text-xs text-gray-400 whitespace-nowrap">
                  Config file
                </td>
                <td className="py-1.5 font-mono">
                  {field.config_file_value != null ? (
                    <span className="inline-flex items-center gap-1.5">
                      <span className="text-gray-900">
                        {field.config_file_value}
                      </span>
                      {tomlLine && (
                        <Link
                          to={`/admin/config?edit#l1:${field.key}`}
                          className="text-gray-400 hover:text-blue-600 transition-colors"
                          title="Edit in config.toml"
                          onClick={onClose}
                        >
                          <ExternalLink className="h-3 w-3" />
                        </Link>
                      )}
                    </span>
                  ) : (
                    <span className="text-gray-300 italic">not set</span>
                  )}
                </td>
              </tr>

              {/* Default */}
              <tr>
                <td className="py-1.5 pr-3 text-xs text-gray-400 whitespace-nowrap">
                  Default
                </td>
                <td className="py-1.5 font-mono text-gray-500">
                  {field.default_value}
                </td>
              </tr>
            </tbody>
          </table>

          {error && (
            <div className="text-xs text-red-600 bg-red-50 rounded-md px-3 py-2">
              {error}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="px-5 py-3 border-t border-gray-100 flex justify-end">
          <button
            onClick={onClose}
            className="rounded-md border border-gray-300 px-4 py-1.5 text-sm font-medium text-gray-600 hover:bg-gray-50 transition-colors"
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
