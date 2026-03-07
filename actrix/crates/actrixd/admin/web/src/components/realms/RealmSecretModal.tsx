import { useEffect } from "react";
import { X, Copy, Download, AlertTriangle, Key } from "lucide-react";

interface RealmSecretModalProps {
  isOpen: boolean;
  realmId: number;
  secret: string;
  previousValidUntil?: number | null;
  onClose: () => void;
}

export function RealmSecretModal({
  isOpen,
  realmId,
  secret,
  previousValidUntil,
  onClose,
}: RealmSecretModalProps) {
  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  function handleCopy() {
    navigator.clipboard.writeText(secret);
  }

  function handleDownload() {
    const blob = new Blob([secret], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `realm-${realmId}-secret.txt`;
    a.click();
    URL.revokeObjectURL(url);
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
      <div className="w-full max-w-md rounded-xl bg-white p-6 shadow-xl">
        <div className="flex items-center justify-between mb-6">
          <h2 className="text-lg font-semibold text-gray-900 flex items-center gap-2">
            <Key className="h-5 w-5 text-amber-500" />
            Secret Rotated
          </h2>
          <button
            onClick={onClose}
            className="rounded-lg p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 transition-colors"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="space-y-6">
          <div className="rounded-lg bg-amber-50 border border-amber-200 p-4">
            <div className="flex items-start gap-3">
              <AlertTriangle className="h-5 w-5 text-amber-600 shrink-0 mt-0.5" />
              <div className="space-y-1">
                <p className="text-sm font-medium text-amber-900">New secret generated</p>
                <p className="text-xs text-amber-700">
                  Save this secret immediately. It will not be shown again.
                </p>
              </div>
            </div>
          </div>

          <div className="space-y-3">
            <div>
              <label className="text-xs font-medium text-gray-500 uppercase">Realm ID</label>
              <p className="text-sm font-mono text-gray-900">{realmId}</p>
            </div>
            
            <div>
              <label className="text-xs font-medium text-gray-500 uppercase">New Secret</label>
              <div className="mt-1 flex items-center gap-2">
                <code className="flex-1 rounded bg-gray-100 px-3 py-2 text-sm font-mono text-gray-800 border border-gray-200 break-all">
                  {secret}
                </code>
              </div>
              <div className="flex gap-2 mt-2">
                <button
                  onClick={handleCopy}
                  className="flex items-center gap-1.5 rounded border border-gray-300 px-3 py-1.5 text-xs font-medium text-gray-700 hover:bg-gray-50 transition-colors"
                >
                  <Copy className="h-3.5 w-3.5" /> Copy
                </button>
                <button
                  onClick={handleDownload}
                  className="flex items-center gap-1.5 rounded border border-gray-300 px-3 py-1.5 text-xs font-medium text-gray-700 hover:bg-gray-50 transition-colors"
                >
                  <Download className="h-3.5 w-3.5" /> Download
                </button>
              </div>
            </div>

            {typeof previousValidUntil === "number" && (
              <div className="rounded bg-blue-50 px-3 py-2 text-xs text-blue-800 border border-blue-100 space-y-1">
                <p><span className="font-semibold">Note:</span> Previous secret remains valid until{" "}
                {new Date(previousValidUntil * 1000).toLocaleString()}.</p>
                <p className="font-medium text-blue-900">Please update your configuration within 4 hours.</p>
              </div>
            )}
          </div>

          <div className="pt-4 border-t border-gray-100 flex justify-end">
            <button
              onClick={onClose}
              className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 transition-colors"
            >
              Close
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
