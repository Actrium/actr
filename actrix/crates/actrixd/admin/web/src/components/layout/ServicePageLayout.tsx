import type { ReactNode } from "react";
import type { ResolvedField } from "../../lib/api";
import { StatusBadge, ConfigFieldsTable } from "../../pages/services/shared";
import { CollapsibleCard } from "../ui/CollapsibleCard";

/* ── Page wrapper ─────────────────────────────────────────── */

export function ServicePageLayout({
  title,
  description,
  headerActions,
  children,
}: {
  title: string;
  description: ReactNode;
  headerActions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-gray-900">{title}</h1>
          <p className="text-sm text-gray-500 mt-1">{description}</p>
        </div>
        {headerActions}
      </div>
      {children}
    </div>
  );
}

/* ── Config section card ──────────────────────────────────── */

export interface ConfigGroup {
  title: string;
  description?: string;
  fields: ResolvedField[];
}

export function ConfigSection({
  title = "Associated configuration",
  storageKey = "config",
  fields,
  groups,
  onRefresh,
}: {
  title?: string;
  /** localStorage key for collapse state */
  storageKey?: string;
  /** Flat field list (simple mode) */
  fields?: ResolvedField[];
  /** Grouped sub-sections (renders sub-headings inside one card) */
  groups?: ConfigGroup[];
  onRefresh?: () => void;
}) {
  const resolvedGroups: ConfigGroup[] = groups
    ?? (fields && fields.length > 0 ? [{ title: "", fields }] : []);

  const nonEmpty = resolvedGroups.filter((g) => g.fields.length > 0);
  if (nonEmpty.length === 0) return null;

  const isSingle = nonEmpty.length === 1 && !nonEmpty[0].title;

  return (
    <CollapsibleCard storageKey={`cfg_${storageKey}`} title={title}>
      {isSingle ? (
        <ConfigFieldsTable fields={nonEmpty[0].fields} onRefresh={onRefresh} />
      ) : (
        <div className="divide-y divide-gray-100">
          {nonEmpty.map((g, i) => (
            <div key={g.title} className={i > 0 ? "pt-5" : ""}>
              <h3 className="mb-1 text-xs font-bold text-gray-900">{g.title}</h3>
              {g.description && (
                <p className="mb-2 text-xs text-gray-400">{g.description}</p>
              )}
              <ConfigFieldsTable fields={g.fields} onRefresh={onRefresh} />
            </div>
          ))}
        </div>
      )}
    </CollapsibleCard>
  );
}

/* ── Status + disabled notice ─────────────────────────────── */

export function StatusSection({
  enabled,
  healthy,
  disabledHint,
}: {
  enabled: boolean;
  healthy?: boolean;
  disabledHint?: ReactNode;
}) {
  return (
    <>
      <div className="flex items-center gap-3">
        <StatusBadge enabled={enabled} healthy={healthy} />
      </div>
      {!enabled && disabledHint && (
        <div className="rounded-xl border border-gray-200 bg-gray-50 p-5 text-sm text-gray-500">
          {disabledHint}
        </div>
      )}
    </>
  );
}
