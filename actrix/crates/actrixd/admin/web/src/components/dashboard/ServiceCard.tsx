import { cn } from "../../lib/utils";

interface ServiceCardProps {
  name: string;
  typeName: string;
  isHealthy: boolean;
  activeConnections: number;
  totalRequests: number;
  averageLatencyMs: number;
  failedRequests?: number;
}

export function ServiceCard({
  name,
  typeName,
  isHealthy,
  activeConnections,
  totalRequests,
  averageLatencyMs,
  failedRequests,
}: ServiceCardProps) {
  return (
    <div className="rounded-xl border border-gray-200 bg-white p-4">
      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm font-medium text-gray-900">{name}</p>
          <p className="text-xs text-gray-500">{typeName}</p>
        </div>
        <span
          className={cn(
            "inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium",
            isHealthy
              ? "bg-green-50 text-green-700"
              : "bg-red-50 text-red-700",
          )}
        >
          {isHealthy ? "Healthy" : "Unhealthy"}
        </span>
      </div>
      <div className={cn(
        "mt-3 grid gap-2 text-center",
        failedRequests && failedRequests > 0 ? "grid-cols-4" : "grid-cols-3",
      )}>
        <div>
          <p className="text-xs text-gray-500">Conns</p>
          <p className="text-sm font-medium text-gray-900">{activeConnections}</p>
        </div>
        <div>
          <p className="text-xs text-gray-500">Requests</p>
          <p className="text-sm font-medium text-gray-900">{totalRequests}</p>
        </div>
        <div>
          <p className="text-xs text-gray-500">Latency</p>
          <p className="text-sm font-medium text-gray-900">
            {averageLatencyMs.toFixed(1)}ms
          </p>
        </div>
        {failedRequests != null && failedRequests > 0 && (
          <div>
            <p className="text-xs text-red-500">Errors</p>
            <p className="text-sm font-medium text-red-600">{failedRequests}</p>
          </div>
        )}
      </div>
    </div>
  );
}
