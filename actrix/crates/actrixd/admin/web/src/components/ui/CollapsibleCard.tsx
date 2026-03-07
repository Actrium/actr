import { useState, useEffect, useCallback, type ReactNode } from "react";
import { ChevronUp, ChevronDown } from "lucide-react";

const STORAGE_PREFIX = "actrix_panel_";

export function CollapsibleCard({
  storageKey,
  title,
  defaultExpanded = true,
  headerRight,
  children,
}: {
  /** Unique key for localStorage persistence */
  storageKey: string;
  title: string;
  /** Initial state when no localStorage value exists */
  defaultExpanded?: boolean;
  /** Optional element rendered between title and chevron */
  headerRight?: ReactNode;
  children: ReactNode;
}) {
  const fullKey = STORAGE_PREFIX + storageKey;

  const [expanded, setExpanded] = useState(() => {
    try {
      const saved = localStorage.getItem(fullKey);
      return saved === null ? defaultExpanded : saved === "1";
    } catch {
      return defaultExpanded;
    }
  });

  useEffect(() => {
    try {
      localStorage.setItem(fullKey, expanded ? "1" : "0");
    } catch {
      // ignore
    }
  }, [expanded, fullKey]);

  const toggle = useCallback(() => setExpanded((v) => !v), []);

  const Icon = expanded ? ChevronUp : ChevronDown;

  return (
    <div className="rounded-xl border border-gray-200 bg-white">
      <div
        className="flex items-center justify-between px-5 py-3 cursor-pointer select-none"
        onClick={toggle}
      >
        <h2 className="text-sm font-semibold text-gray-700">{title}</h2>
        {headerRight && <div className="flex-1 flex justify-end mr-2">{headerRight}</div>}
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            toggle();
          }}
          className="rounded p-0.5 text-gray-400 hover:text-gray-600 hover:bg-gray-100 transition-colors"
          aria-label={expanded ? "Collapse" : "Expand"}
        >
          <Icon size={16} />
        </button>
      </div>
      {expanded && <div className="px-5 pb-5">{children}</div>}
    </div>
  );
}
