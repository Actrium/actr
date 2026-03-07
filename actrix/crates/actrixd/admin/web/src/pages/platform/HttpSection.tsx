import { usePlatformData, filterFields, PlatformDataGuard } from "./shared";
import { ConfigFieldsTable } from "../services/shared";

export function HttpSection() {
  const { data, error, fetchData } = usePlatformData();

  return (
    <PlatformDataGuard data={data} error={error}>
      {(d) => (
        <div className="space-y-6">
          <div>
            <h1 className="text-xl font-semibold text-gray-900">HTTP Binding</h1>
            <p className="text-sm text-gray-500 mt-1">
              Listen address, port, and advertised endpoint for HTTP
            </p>
          </div>
          <div className="rounded-xl border border-gray-200 bg-white p-5">
            <h2 className="mb-3 text-sm font-semibold text-gray-700">Configuration</h2>
            <ConfigFieldsTable
              fields={filterFields(d.config_fields, ["bind.http"])}
              onRefresh={fetchData}
            />
          </div>
        </div>
      )}
    </PlatformDataGuard>
  );
}
