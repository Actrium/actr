import { usePlatformData, filterFields, PlatformDataGuard } from "./shared";
import { ConfigFieldsTable } from "../services/shared";

const KEYS = ["enable", "name", "env", "location_tag", "sqlite_path"];

export function NodeSection() {
  const { data, error, fetchData } = usePlatformData();

  return (
    <PlatformDataGuard data={data} error={error}>
      {(d) => (
        <div className="space-y-6">
          <div>
            <h1 className="text-xl font-semibold text-gray-900">Node Identity</h1>
            <p className="text-sm text-gray-500 mt-1">
              Instance name, environment, and database path
            </p>
          </div>
          <div className="rounded-xl border border-gray-200 bg-white p-5">
            <h2 className="mb-3 text-sm font-semibold text-gray-700">Configuration</h2>
            <ConfigFieldsTable
              fields={filterFields(d.config_fields, KEYS)}
              onRefresh={fetchData}
            />
          </div>
        </div>
      )}
    </PlatformDataGuard>
  );
}
