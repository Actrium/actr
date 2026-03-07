import { cn } from "../../lib/utils";

interface MetricCardProps {
  label: string;
  value: string;
  percent?: number;
  subtitle?: string;
}

export function MetricCard({ label, value, percent, subtitle }: MetricCardProps) {
  return (
    <div className="rounded-xl border border-gray-200 bg-white p-4">
      <p className="text-xs font-medium text-gray-500">{label}</p>
      <p className="mt-1 text-lg font-semibold text-gray-900">{value}</p>
      {subtitle && (
        <p className="mt-0.5 text-xs text-gray-500">{subtitle}</p>
      )}
      {percent != null && (
        <div className="mt-3">
          <div className="h-1.5 w-full rounded-full bg-gray-100">
            <div
              className={cn(
                "h-1.5 rounded-full transition-all",
                percent > 90
                  ? "bg-red-500"
                  : percent > 70
                    ? "bg-yellow-500"
                    : "bg-blue-500",
              )}
              style={{ width: `${Math.min(100, percent)}%` }}
            />
          </div>
        </div>
      )}
    </div>
  );
}
