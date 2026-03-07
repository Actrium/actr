import { useEffect, useState, useCallback } from "react";
import { api, type PlatformDetail, type ResolvedField } from "../../lib/api";

/** Hook: fetch platform detail with 5s polling, return data + error + refresh */
export function usePlatformData() {
  const [data, setData] = useState<PlatformDetail | null>(null);
  const [error, setError] = useState("");

  const fetchData = useCallback(async () => {
    try {
      const d = await api.getPlatformDetail();
      setData(d);
      setError("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load");
    }
  }, []);

  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 5000);
    return () => clearInterval(interval);
  }, [fetchData]);

  return { data, error, fetchData };
}

/** Filter config_fields by key prefix */
export function filterFields(
  fields: ResolvedField[],
  prefixes: string[],
): ResolvedField[] {
  return fields.filter((f) =>
    prefixes.some((p) => f.key === p || f.key.startsWith(p + ".")),
  );
}

/** Loading / error wrapper */
export function PlatformDataGuard({
  data,
  error,
  children,
}: {
  data: PlatformDetail | null;
  error: string;
  children: (data: PlatformDetail) => React.ReactNode;
}) {
  if (error && !data) {
    return (
      <div className="rounded-lg border border-red-200 bg-red-50 p-4 text-sm text-red-700">
        {error}
      </div>
    );
  }
  if (!data) {
    return <div className="text-sm text-gray-500">Loading...</div>;
  }
  return <>{children(data)}</>;
}
