import { useState, useEffect, useRef, useCallback } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { Copy, Check, ChevronDown, X, Minus, Plus } from "lucide-react";
import { parse as parseToml } from "smol-toml";
import { highlightToml } from "../lib/highlight";
import { api, type ConfigFieldDef, type ConfigOverrideEntry, type ResolvedField } from "../lib/api";

/** Parse hash like #l1:bind.http.port → { panel, key }. Works for l0, l1, l2. */
function parsePanelKeyHash(hash: string): { panel: string; key: string } | null {
  const m = hash.match(/^#(l[012]):(.+)$/);
  return m ? { panel: m[1], key: m[2] } : null;
}

/** Find 1-based line number of a dotted config key in TOML content. */
function findTomlLine(content: string, key: string): number | null {
  const lines = content.split("\n");
  let currentSection: string[] = [];

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();

    // Section header: [bind.http]
    const sec = line.match(/^\[([^\]]+)\]$/);
    if (sec) {
      currentSection = sec[1].split(".");
      continue;
    }

    // Key assignment: port = 8080
    const kv = line.match(/^([\w][\w-]*)\s*=/);
    if (kv) {
      const fullKey = [...currentSection, kv[1]].join(".");
      if (fullKey === key) return i + 1;
    }
  }
  return null;
}

/** Strip all inline styles from shiki output so Tailwind classes take over. */
function stripShikiStyles(el: HTMLElement | null) {
  if (!el) return;
  const pre = el.querySelector("pre");
  if (pre) {
    pre.removeAttribute("style");
    const code = pre.querySelector("code");
    if (code) code.removeAttribute("style");
  }
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/** Compare immutable config fields between original and edited TOML. */
function checkRestartRequired(
  original: string,
  edited: string,
): { needed: boolean; reasons: string[] } {
  try {
    const oldCfg = parseToml(original) as Record<string, any>;
    const newCfg = parseToml(edited) as Record<string, any>;
    const reasons: string[] = [];
    const eq = (a: any, b: any) => JSON.stringify(a) === JSON.stringify(b);

    if (!eq((oldCfg.bind as any)?.http, (newCfg.bind as any)?.http))
      reasons.push("bind.http");
    if (oldCfg.sqlite_path !== newCfg.sqlite_path)
      reasons.push("sqlite_path");
    if (!eq(oldCfg.recording, newCfg.recording)) reasons.push("recording");
    if (oldCfg.env !== newCfg.env) reasons.push("env");

    return { needed: reasons.length > 0, reasons };
  } catch {
    return { needed: false, reasons: [] };
  }
}

/* ── Accordion section header ─────────────────────────────── */

const LAYER_STYLES: Record<string, { bg: string; bgOpen: string; label: string; desc: string }> = {
  eff: { bg: "bg-emerald-50 hover:bg-emerald-100/80", bgOpen: "bg-emerald-100", label: "text-emerald-800", desc: "text-emerald-500" },
  l2: { bg: "bg-purple-50 hover:bg-purple-100/80", bgOpen: "bg-purple-100", label: "text-purple-800", desc: "text-purple-500" },
  l1: { bg: "bg-indigo-50 hover:bg-indigo-100/80", bgOpen: "bg-indigo-100", label: "text-indigo-800", desc: "text-indigo-500" },
  l0: { bg: "bg-gray-100 hover:bg-gray-200/80", bgOpen: "bg-gray-200", label: "text-gray-700", desc: "text-gray-500" },
};

function SectionHeader({
  id,
  title,
  description,
  open,
  onToggle,
  badge,
}: {
  id: string;
  title: string;
  description: string;
  open: boolean;
  onToggle: () => void;
  badge?: number;
}) {
  const s = LAYER_STYLES[id] ?? LAYER_STYLES.l0;
  return (
    <button
      onClick={onToggle}
      className={`w-full flex items-center justify-between px-4 py-3 text-left transition-colors ${open ? s.bgOpen : s.bg}`}
    >
      <div className="flex items-center gap-3">
        <span className={`text-sm font-semibold ${s.label}`}>{title}</span>
        {badge !== undefined && badge > 0 && (
          <span className={`rounded-full px-2 py-0.5 text-[10px] font-semibold ${
            id === "eff" ? "bg-emerald-100 text-emerald-600" : "bg-purple-100 text-purple-600"
          }`}>
            {badge}
          </span>
        )}
        <span className={`text-xs ${s.desc}`}>{description}</span>
      </div>
      {open ? (
        <Minus className={`h-4 w-4 ${s.label}`} />
      ) : (
        <Plus className={`h-4 w-4 ${s.label}`} />
      )}
    </button>
  );
}

/* ── Main component ───────────────────────────────────────── */

export function ConfigEditor() {
  /* Accordion state — exclusive: only one panel open at a time */
  const [openSection, setOpenSection] = useState<string | null>("eff");

  const toggleSection = (id: string) => {
    setOpenSection((prev) => (prev === id ? null : id));
  };

  /* L2: Overrides */
  const [overrides, setOverrides] = useState<ConfigOverrideEntry[]>([]);
  const [overridesLoading, setOverridesLoading] = useState(true);

  /* L0: Defaults */
  const [registry, setRegistry] = useState<ConfigFieldDef[]>([]);
  const [registryLoading, setRegistryLoading] = useState(true);

  /* L1: config.toml editor state (preserved from original) */
  const [originalContent, setOriginalContent] = useState("");
  const [diskContent, setDiskContent] = useState("");
  const [editContent, setEditContent] = useState("");
  const [configPath, setConfigPath] = useState("");
  const [loading, setLoading] = useState(true);
  const [toasts, setToasts] = useState<
    { id: number; type: "error" | "success" | "info"; text: string }[]
  >([]);
  const toastId = useRef(0);
  const [editing, setEditing] = useState(false);
  const [highlightedHtml, setHighlightedHtml] = useState("");
  const [editHighlightedHtml, setEditHighlightedHtml] = useState("");
  const [selectedLines, setSelectedLines] = useState<[number, number] | null>(
    null,
  );
  const [copied, setCopied] = useState(false);
  const [saving, setSaving] = useState(false);
  const [applying, setApplying] = useState(false);
  const [dropdownOpen, setDropdownOpen] = useState(false);

  /* Cross-panel navigation highlight */
  const [highlightKey, setHighlightKey] = useState<string | null>(null);
  const highlightTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  function toast(
    type: "error" | "success" | "info",
    text: string,
    ms = 4000,
  ) {
    const id = ++toastId.current;
    setToasts((prev) => [...prev, { id, type, text }]);
    if (ms > 0) setTimeout(() => dismissToast(id), ms);
    return id;
  }
  function dismissToast(id: number) {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const highlightRef = useRef<HTMLDivElement>(null);
  const lineHighlightRef = useRef<HTMLDivElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const location = useLocation();
  const navigate = useNavigate();

  // Derived state
  const hasUnsavedChanges = editing && editContent !== diskContent;
  const hasChangesFromOriginal = editing && editContent !== originalContent;
  const restartCheck = hasChangesFromOriginal
    ? checkRestartRequired(originalContent, editContent)
    : { needed: false, reasons: [] };
  const primaryIsRestart = restartCheck.needed;

  // Close dropdown on outside click
  useEffect(() => {
    if (!dropdownOpen) return;
    function handleClickOutside(e: MouseEvent) {
      if (
        dropdownRef.current &&
        !dropdownRef.current.contains(e.target as Node)
      ) {
        setDropdownOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [dropdownOpen]);

  const pendingEditScroll = useRef<[number, number] | null>(null);
  useEffect(() => {
    if (!editing || !pendingEditScroll.current) return;
    const target = pendingEditScroll.current;
    pendingEditScroll.current = null;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        const ta = textareaRef.current;
        if (!ta) return;
        ta.focus();
        const lineH = 22;
        const scrollTarget = (target[0] - 1) * lineH;
        const viewH = ta.clientHeight;
        ta.scrollTop = Math.max(0, scrollTarget - viewH / 2);
        syncScroll();
      });
    });
  }, [editing]);

  // Auto-enter edit mode when ?edit is in the URL
  const wantsAutoEdit = useRef(
    new URLSearchParams(location.search).has("edit"),
  );
  useEffect(() => {
    if (wantsAutoEdit.current && !loading && diskContent && !editing) {
      wantsAutoEdit.current = false;
      if (selectedLines) {
        pendingEditScroll.current = selectedLines;
      }
      handleEdit();
      navigate(location.pathname + location.hash, { replace: true });
    }
  }, [loading, diskContent]);

  // Load all data on mount
  useEffect(() => {
    loadConfig();
    api.getOverrides()
      .then(setOverrides)
      .catch(() => {})
      .finally(() => setOverridesLoading(false));
    api.getRegistry()
      .then(setRegistry)
      .catch(() => {})
      .finally(() => setRegistryLoading(false));
  }, []);

  // Auto-open L1 when ?edit or #l1: hash is present
  useEffect(() => {
    if (!loading && (wantsAutoEdit.current || location.hash.startsWith("#l1:"))) {
      setOpenSection("l1");
    }
  }, [loading]);

  // Handle #l{0,1,2}:key hash → open panel + highlight/scroll
  useEffect(() => {
    const pk = parsePanelKeyHash(location.hash);
    if (!pk) return;

    setOpenSection(pk.panel);

    if (pk.panel === "l1") {
      // L1: find TOML line and scroll the code view
      const content = diskContent || originalContent;
      const line = findTomlLine(content, pk.key);
      if (line) {
        setSelectedLines([line, line]);
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            const el = document.getElementById(`L${line}`);
            el?.scrollIntoView({ block: "center", behavior: "smooth" });
          });
        });
      }
    } else {
      // L0 / L2: highlight the table row
      setHighlightKey(pk.key);
      clearTimeout(highlightTimer.current);
      highlightTimer.current = setTimeout(() => setHighlightKey(null), 3000);
      requestAnimationFrame(() => {
        const el = document.getElementById(`row-${pk.panel}-${pk.key}`);
        el?.scrollIntoView({ block: "center", behavior: "smooth" });
      });
    }
  }, [location.hash, diskContent, originalContent]);

  /** Navigate from Effective source badge to the originating panel. */
  function navigateToSource(field: ResolvedField) {
    const panel = field.source === "config_file" ? "l1" : field.source === "override" ? "l2" : "l0";
    navigate(`#${panel}:${field.key}`);
  }

  // Syntax highlighting for view mode
  useEffect(() => {
    const viewContent = diskContent || originalContent;
    if (viewContent && !editing) {
      highlightToml(viewContent).then(setHighlightedHtml);
    }
  }, [originalContent, diskContent, editing]);

  // Syntax highlighting for edit mode
  useEffect(() => {
    if (editing && editContent) {
      highlightToml(editContent).then(setEditHighlightedHtml);
    }
  }, [editing, editContent]);

  const syncScroll = useCallback(() => {
    if (textareaRef.current && highlightRef.current) {
      highlightRef.current.scrollTop = textareaRef.current.scrollTop;
      highlightRef.current.scrollLeft = textareaRef.current.scrollLeft;
    }
    if (textareaRef.current && lineHighlightRef.current) {
      lineHighlightRef.current.scrollTop = textareaRef.current.scrollTop;
    }
  }, []);

  async function loadConfig() {
    setLoading(true);
    try {
      const data = await api.getConfigFile();
      setOriginalContent(data.content);
      setDiskContent(data.content);
      setConfigPath(data.path);
    } catch (err) {
      toast("error", err instanceof Error ? err.message : "Failed to load config", 0);
    } finally {
      setLoading(false);
    }
  }

  function handleEdit() {
    setEditContent(diskContent || originalContent);
    setEditing(true);
  }

  function handleCancel() {
    setEditing(false);
    setEditContent("");
  }

  async function handleSave() {
    setSaving(true);
    try {
      const result = await api.saveConfigFile(editContent);
      if (!result.saved) {
        toast("error", result.error || "Failed to save");
        return;
      }
      setDiskContent(editContent);
      toast("success", "Saved");
    } catch (err) {
      toast("error", err instanceof Error ? err.message : "Failed to save");
    } finally {
      setSaving(false);
    }
  }

  async function handleReload() {
    setApplying(true);
    setDropdownOpen(false);
    try {
      await api.reloadNode();
      toast("success", "Reload signal sent");
      setTimeout(async () => {
        await loadConfig();
        setEditing(false);
      }, 1500);
    } catch (err) {
      toast("error", err instanceof Error ? err.message : "Reload failed");
    } finally {
      setApplying(false);
    }
  }

  function pollUntilReady(infoId: number) {
    let attempts = 0;
    const maxAttempts = 30;
    const iv = setInterval(async () => {
      attempts++;
      try {
        const res = await fetch("/admin/health");
        if (res.ok) {
          clearInterval(iv);
          window.location.reload();
        }
      } catch {
        // still down
      }
      if (attempts >= maxAttempts) {
        clearInterval(iv);
        dismissToast(infoId);
        toast("error", "Service did not come back within 30s — try refreshing manually", 0);
        setApplying(false);
      }
    }, 1000);
  }

  async function handleRestart() {
    setApplying(true);
    setDropdownOpen(false);
    try {
      await api.restartNode();
    } catch {
      // Connection reset is expected during restart
    }
    const id = toast("info", "Restarting...", 0);
    pollUntilReady(id);
  }

  function handleCopyPath() {
    navigator.clipboard.writeText(configPath);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  function handleLineClick(lineNum: number, e: React.MouseEvent) {
    if (editing) return;
    if (e.shiftKey && selectedLines) {
      setSelectedLines([
        Math.min(selectedLines[0], lineNum),
        Math.max(selectedLines[1], lineNum),
      ]);
    } else {
      setSelectedLines([lineNum, lineNum]);
    }
  }

  function isLineSelected(lineNum: number): boolean {
    if (!selectedLines) return false;
    return lineNum >= selectedLines[0] && lineNum <= selectedLines[1];
  }

  const displayContent = editing
    ? editContent
    : diskContent || originalContent;
  const lineCount = displayContent.split("\n").length;
  const gutterText = Array.from({ length: lineCount }, (_, i) => i + 1).join(
    "\n",
  );
  const CODE_CLS = "font-mono text-sm leading-[22px]";

  // Merge effective view from L0 (registry) + L1 (TOML) + L2 (overrides)
  const effectiveLoading = registryLoading || overridesLoading || loading;
  const effectiveFields = (() => {
    if (effectiveLoading) return [];

    // Parse L1 TOML into flat key→value map
    const tomlContent = diskContent || originalContent;
    let tomlFlat: Record<string, string> = {};
    try {
      const parsed = parseToml(tomlContent);
      const flatten = (obj: Record<string, any>, prefix = "") => {
        for (const [k, v] of Object.entries(obj)) {
          const key = prefix ? `${prefix}.${k}` : k;
          if (v !== null && typeof v === "object" && !Array.isArray(v)) {
            flatten(v, key);
          } else {
            tomlFlat[key] = Array.isArray(v) ? JSON.stringify(v) : String(v);
          }
        }
      };
      flatten(parsed);
    } catch { /* invalid TOML — ignore L1 values */ }

    const overrideMap = new Map(overrides.map((o) => [o.key_path, o.value]));

    // Collect all keys from all three sources
    const allKeys = new Set([
      ...registry.map((r) => r.key),
      ...Object.keys(tomlFlat),
      ...overrides.map((o) => o.key_path),
    ]);

    const regMap = new Map(registry.map((r) => [r.key, r]));

    return [...allKeys].sort().map((key) => {
      const reg = regMap.get(key);
      const l0 = reg?.default_value ?? "";
      const l1 = tomlFlat[key] ?? null;
      const l2 = overrideMap.get(key) ?? null;

      const effective = l2 ?? l1 ?? l0;
      const source: "override" | "config_file" | "default" =
        l2 != null ? "override" : l1 != null ? "config_file" : "default";

      return {
        key,
        effective_value: effective,
        source,
        description: reg?.description ?? "",
        value_type: reg?.value_type ?? "",
        dynamic: reg?.dynamic ?? false,
        reloadable: reg?.reloadable ?? false,
        default_value: l0,
        config_file_value: l1,
        override_value: l2,
      } as ResolvedField;
    });
  })();

  const applyEnabled =
    !hasUnsavedChanges && hasChangesFromOriginal && !applying;

  const toastColors = {
    error: "border-red-200 bg-red-50 text-red-700",
    success: "border-green-200 bg-green-50 text-green-700",
    info: "border-blue-200 bg-blue-50 text-blue-700",
  };

  return (
    <div>
      {/* Toast container */}
      {toasts.length > 0 && (
        <div className="fixed top-4 left-1/2 -translate-x-1/2 z-50 flex flex-col gap-2 w-80">
          {toasts.map((t) => (
            <div
              key={t.id}
              className={`flex items-start gap-2 rounded-lg border px-3 py-2 shadow-md text-sm ${toastColors[t.type]}`}
            >
              <span className="flex-1">{t.text}</span>
              <button
                onClick={() => dismissToast(t.id)}
                className="shrink-0 opacity-60 hover:opacity-100"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          ))}
        </div>
      )}

      <div className="space-y-3">
        {/* ═══ Effective (resolved) ═══ */}
        <div className="rounded-xl border border-gray-200 bg-white overflow-hidden">
          <SectionHeader
            id="eff"
            title="Effective"
            description="= L2 + L1 + L0"
            open={openSection === "eff"}
            onToggle={() => toggleSection("eff")}
            badge={effectiveFields.length}
          />
          {openSection === "eff" && (
            <div className="border-t border-gray-200">
              {effectiveLoading ? (
                <div className="px-4 py-6 text-center text-sm text-gray-400">Loading...</div>
              ) : effectiveFields.length === 0 ? (
                <div className="px-4 py-6 text-center text-sm text-gray-400">
                  No config fields resolved.
                </div>
              ) : (
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-gray-100 text-left text-xs text-gray-400">
                      <th className="px-4 py-2 font-medium">Key</th>
                      <th className="px-4 py-2 font-medium">Value</th>
                      <th className="px-4 py-2 font-medium">Source</th>
                      <th className="px-4 py-2 font-medium hidden lg:table-cell">Description</th>
                    </tr>
                  </thead>
                  <tbody>
                    {effectiveFields.map((f) => {
                      const srcColors: Record<string, string> = {
                        override: "bg-purple-50 text-purple-600",
                        config_file: "bg-indigo-50 text-indigo-600",
                        default: "bg-gray-100 text-gray-500",
                      };
                      return (
                        <tr key={f.key} className="border-b border-gray-50 hover:bg-gray-50/50">
                          <td className="px-4 py-2 font-mono text-gray-700">{f.key}</td>
                          <td className="px-4 py-2 font-mono text-gray-900">{f.effective_value}</td>
                          <td className="px-4 py-2">
                            <button
                              onClick={() => navigateToSource(f)}
                              className={`rounded px-1.5 py-0.5 text-[10px] font-medium cursor-pointer hover:ring-1 hover:ring-current transition-shadow ${srcColors[f.source] ?? srcColors.default}`}
                            >
                              {f.source === "config_file" ? "L1 toml" : f.source === "override" ? "L2 override" : "L0 default"}
                            </button>
                          </td>
                          <td className="px-4 py-2 text-xs text-gray-400 hidden lg:table-cell">{f.description}</td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              )}
            </div>
          )}
        </div>

        {/* ═══ L2 Overrides ═══ */}
        <div className="rounded-xl border border-gray-200 bg-white overflow-hidden">
          <SectionHeader
            id="l2"
            title="L2  Overrides"
            description="dynamic conf stored in SQLite"
            open={openSection === "l2"}
            onToggle={() => toggleSection("l2")}
            badge={overrides.length}
          />
          {openSection === "l2" && (
            <div className="border-t border-gray-200">
              {overridesLoading ? (
                <div className="px-4 py-6 text-center text-sm text-gray-400">Loading...</div>
              ) : overrides.length === 0 ? (
                <div className="px-4 py-6 text-center text-sm text-gray-400">
                  No runtime overrides set.
                </div>
              ) : (
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-gray-100 text-left text-xs text-gray-400">
                      <th className="px-4 py-2 font-medium">Key</th>
                      <th className="px-4 py-2 font-medium">Value</th>
                      <th className="px-4 py-2 font-medium">Updated</th>
                    </tr>
                  </thead>
                  <tbody>
                    {overrides.map((o) => (
                      <tr
                        key={o.key_path}
                        id={`row-l2-${o.key_path}`}
                        className={`border-b border-gray-50 transition-colors duration-700 ${
                          highlightKey === o.key_path ? "bg-purple-100" : "hover:bg-gray-50/50"
                        }`}
                      >
                        <td className="px-4 py-2 font-mono text-purple-700">{o.key_path}</td>
                        <td className="px-4 py-2 font-mono text-gray-900">{o.value}</td>
                        <td className="px-4 py-2 text-xs text-gray-400">{o.updated_at}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          )}
        </div>

        {/* ═══ L1 config.toml ═══ */}
        <div className="rounded-xl border border-gray-200 bg-white overflow-hidden">
          <SectionHeader
            id="l1"
            title="L1  config.toml"
            description={configPath ? `hot-reload via SIGHUP` : "loading..."}
            open={openSection === "l1"}
            onToggle={() => toggleSection("l1")}
          />
          {openSection === "l1" && (
            <div className="border-t border-gray-200">
              {loading ? (
                <div className="px-4 py-6 text-center text-sm text-gray-400">
                  Loading configuration...
                </div>
              ) : (
                <>
                  {/* Toolbar */}
                  <div className="flex items-center justify-between border-b border-gray-200 px-4 py-2.5">
                    <div className="flex items-center gap-2">
                      <span className={`${CODE_CLS} text-gray-700`}>
                        {editing ? (
                          <span className="text-blue-600">
                            config.toml
                            {hasUnsavedChanges && (
                              <span className="text-amber-500">*</span>
                            )}
                          </span>
                        ) : (
                          <>
                            <span className="text-blue-600">
                              {configPath || "config.toml"}
                            </span>
                          </>
                        )}
                      </span>
                      {configPath && (
                        <button
                          onClick={handleCopyPath}
                          className="text-gray-400 hover:text-gray-600 transition-colors"
                          title="Copy path"
                        >
                          {copied ? (
                            <Check className="h-3.5 w-3.5 text-green-500" />
                          ) : (
                            <Copy className="h-3.5 w-3.5" />
                          )}
                        </button>
                      )}
                    </div>

                    <div className="flex items-center gap-2">
                      {editing ? (
                        <>
                          <button
                            onClick={handleCancel}
                            className="rounded-lg border border-gray-300 bg-white px-3 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50 transition-colors"
                          >
                            Cancel
                          </button>
                          <button
                            onClick={handleSave}
                            disabled={!hasUnsavedChanges || saving}
                            className="rounded-lg bg-blue-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                          >
                            {saving ? "Saving..." : "Save"}
                          </button>

                          {/* Reload / Restart split button */}
                          <div ref={dropdownRef} className="relative inline-flex">
                            <button
                              onClick={primaryIsRestart ? handleRestart : handleReload}
                              disabled={!applyEnabled}
                              className={`rounded-l-lg px-3 py-1.5 text-sm font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed ${
                                primaryIsRestart
                                  ? "bg-amber-600 text-white hover:bg-amber-700"
                                  : "bg-emerald-600 text-white hover:bg-emerald-700"
                              }`}
                              title={
                                hasUnsavedChanges
                                  ? "Save changes first"
                                  : restartCheck.reasons.length > 0
                                    ? `Restart required: ${restartCheck.reasons.join(", ")}`
                                    : "Reload configuration"
                              }
                            >
                              {applying
                                ? "..."
                                : primaryIsRestart
                                  ? "Restart"
                                  : "Reload"}
                            </button>
                            <button
                              onClick={() => setDropdownOpen(!dropdownOpen)}
                              disabled={!applyEnabled}
                              className={`rounded-r-lg border-l border-white/30 px-1.5 py-1.5 text-sm transition-colors disabled:opacity-50 disabled:cursor-not-allowed ${
                                primaryIsRestart
                                  ? "bg-amber-600 text-white hover:bg-amber-700"
                                  : "bg-emerald-600 text-white hover:bg-emerald-700"
                              }`}
                            >
                              <ChevronDown className="h-3.5 w-3.5" />
                            </button>
                            {dropdownOpen && (
                              <div className="absolute right-0 top-full mt-1 w-36 rounded-lg border border-gray-200 bg-white shadow-lg z-20 py-1">
                                {primaryIsRestart ? (
                                  <button
                                    disabled
                                    className="w-full px-3 py-1.5 text-left text-sm text-gray-400 cursor-not-allowed"
                                  >
                                    Reload
                                  </button>
                                ) : (
                                  <button
                                    onClick={handleRestart}
                                    className="w-full px-3 py-1.5 text-left text-sm text-gray-700 hover:bg-gray-50"
                                  >
                                    Restart
                                  </button>
                                )}
                              </div>
                            )}
                          </div>
                        </>
                      ) : (
                        <button
                          onClick={handleEdit}
                          className="rounded-lg border border-gray-300 bg-white px-3 py-1.5 text-sm font-medium text-gray-700 hover:bg-gray-50 transition-colors"
                        >
                          Edit
                        </button>
                      )}
                    </div>
                  </div>

                  {/* Code area */}
                  <div className="flex bg-gray-50">
                    {/* Line number gutter */}
                    <div className="relative border-r border-gray-200 select-none">
                      <div className={`absolute inset-0 z-10 py-4 pl-4 pr-3 ${editing ? "pointer-events-none" : ""}`}>
                        {Array.from({ length: lineCount }, (_, i) => {
                          const num = i + 1;
                          return (
                            <div
                              key={num}
                              id={editing ? undefined : `L${num}`}
                              className={`h-[22px] ${!editing ? "cursor-pointer" : ""} ${
                                isLineSelected(num)
                                  ? "bg-amber-100/60 -ml-4 pl-4 -mr-3 pr-3"
                                  : ""
                              }`}
                              onClick={editing ? undefined : (e) => handleLineClick(num, e)}
                            />
                          );
                        })}
                      </div>
                      <pre
                        className={`${CODE_CLS} m-0 py-4 pl-4 pr-3 text-right text-gray-400`}
                      >
                        {gutterText}
                      </pre>
                    </div>

                    {editing ? (
                      <div className="relative flex-1 min-h-[600px]">
                        {selectedLines && (
                          <div
                            ref={lineHighlightRef}
                            className="absolute inset-0 pointer-events-none py-4 z-10 overflow-hidden"
                            aria-hidden
                          >
                            {Array.from({ length: lineCount }, (_, i) => (
                              <div
                                key={i}
                                className={`h-[22px] ${isLineSelected(i + 1) ? "bg-amber-100/60" : ""}`}
                              />
                            ))}
                          </div>
                        )}
                        <div
                          ref={highlightRef}
                          className="absolute inset-0 overflow-hidden pointer-events-none"
                          aria-hidden
                        >
                          <div
                            className={`${CODE_CLS} [&_pre]:!m-0 [&_pre]:!p-4 [&_pre]:!bg-transparent [&_pre]:!rounded-none`}
                            ref={stripShikiStyles}
                            dangerouslySetInnerHTML={
                              editHighlightedHtml
                                ? { __html: editHighlightedHtml }
                                : {
                                    __html: `<pre class="${CODE_CLS} m-0 p-4 whitespace-pre bg-transparent">${escapeHtml(editContent)}</pre>`,
                                  }
                            }
                          />
                        </div>
                        <textarea
                          ref={textareaRef}
                          value={editContent}
                          onChange={(e) => setEditContent(e.target.value)}
                          onScroll={syncScroll}
                          className={`${CODE_CLS} relative w-full h-full min-h-[600px] p-4 text-transparent caret-gray-900 bg-transparent border-0 focus:outline-none resize-none`}
                          spellCheck={false}
                        />
                      </div>
                    ) : (
                      <div className="flex-1 relative overflow-x-auto">
                        {selectedLines && (
                          <div
                            className="absolute inset-0 pointer-events-none py-4"
                            aria-hidden
                          >
                            {Array.from({ length: lineCount }, (_, i) => (
                              <div
                                key={i}
                                className={`h-[22px] ${isLineSelected(i + 1) ? "bg-amber-100/60" : ""}`}
                              />
                            ))}
                          </div>
                        )}
                        <div
                          className={`${CODE_CLS} relative [&_pre]:!m-0 [&_pre]:!p-4 [&_pre]:!bg-transparent [&_pre]:!rounded-none`}
                          ref={stripShikiStyles}
                          dangerouslySetInnerHTML={
                            highlightedHtml
                              ? { __html: highlightedHtml }
                              : {
                                  __html: `<pre class="${CODE_CLS} m-0 p-4 whitespace-pre text-gray-900 bg-transparent">${escapeHtml(displayContent)}</pre>`,
                                }
                          }
                        />
                      </div>
                    )}
                  </div>
                </>
              )}
            </div>
          )}
        </div>

        {/* ═══ L0 Defaults ═══ */}
        <div className="rounded-xl border border-gray-200 bg-white overflow-hidden">
          <SectionHeader
            id="l0"
            title="L0  Defaults"
            description="built into binary"
            open={openSection === "l0"}
            onToggle={() => toggleSection("l0")}
            badge={registry.length}
          />
          {openSection === "l0" && (
            <div className="border-t border-gray-200">
              {registryLoading ? (
                <div className="px-4 py-6 text-center text-sm text-gray-400">Loading...</div>
              ) : registry.length === 0 ? (
                <div className="px-4 py-6 text-center text-sm text-gray-400">
                  No registry entries.
                </div>
              ) : (
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-gray-100 text-left text-xs text-gray-400">
                      <th className="px-4 py-2 font-medium">Key</th>
                      <th className="px-4 py-2 font-medium">Default</th>
                      <th className="px-4 py-2 font-medium">Type</th>
                      <th className="px-4 py-2 font-medium hidden lg:table-cell">Description</th>
                    </tr>
                  </thead>
                  <tbody>
                    {registry.map((r) => (
                      <tr
                        key={r.key}
                        id={`row-l0-${r.key}`}
                        className={`border-b border-gray-50 transition-colors duration-700 ${
                          highlightKey === r.key ? "bg-gray-200" : "hover:bg-gray-50/50"
                        }`}
                      >
                        <td className="px-4 py-2 font-mono text-gray-700">{r.key}</td>
                        <td className="px-4 py-2 font-mono text-gray-500">{r.default_value}</td>
                        <td className="px-4 py-2">
                          <span className="inline-flex items-center gap-1.5">
                            <span className="text-xs text-gray-400">{r.value_type}</span>
                            {r.dynamic && (
                              <span className="rounded bg-green-50 px-1 py-0.5 text-[10px] text-green-600">dyn</span>
                            )}
                            {r.reloadable && (
                              <span className="rounded bg-blue-50 px-1 py-0.5 text-[10px] text-blue-600">reload</span>
                            )}
                          </span>
                        </td>
                        <td className="px-4 py-2 text-xs text-gray-400 hidden lg:table-cell">{r.description}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
