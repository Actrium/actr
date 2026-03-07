import { useState, type FormEvent, useEffect } from "react";
import { api, type RealmInfo, type RealmMutationResponse } from "../../lib/api";
import { X, Copy, Download, CheckCircle, AlertTriangle } from "lucide-react";

interface RealmFormProps {
  realm: RealmInfo | null;
  existingRealmIds?: Set<number>;
  onSubmit: (data: { realm_id: number; name: string; enabled: boolean }) => Promise<RealmMutationResponse | void>;
  onClose: () => void;
  onRefresh?: () => void;
}

export function RealmForm({ realm, existingRealmIds, onSubmit, onClose, onRefresh }: RealmFormProps) {
  const isEditing = !!realm;
  
  // Auto-generate ID if creating new (Range: 1,000,000,000 to u32::MAX)
  // Ensure uniqueness against existing IDs
  const [realmId, setRealmId] = useState(() => {
    if (realm?.realm_id) return realm.realm_id.toString();
    
    // Attempt to generate a unique ID
    for (let i = 0; i < 100; i++) {
      const newId = Math.floor(Math.random() * (4294967295 - 1000000000 + 1) + 1000000000);
      if (!existingRealmIds?.has(newId)) {
        return newId.toString();
      }
    }
    // Fallback if loop fails (extremely unlikely given the range)
    return Math.floor(Math.random() * (4294967295 - 1000000000 + 1) + 1000000000).toString();
  });
  const [name, setName] = useState(realm?.name ?? "");
  const [enabled, setEnabled] = useState(realm?.enabled ?? true);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  
  // Wizard state
  const [step, setStep] = useState<"input" | "success">("input");
  const [createdSecret, setCreatedSecret] = useState<string | null>(null);
  const [createdRealm, setCreatedRealm] = useState<{ id: number; name: string } | null>(null);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      } else if (e.key === "Enter" && step === "success") {
        e.preventDefault();
        handleActivate();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose, step, createdRealm]); // Add step dependency

  // If we are in creation mode (no realm prop), force enabled=false initially
  // The user will choose to activate or keep stored in the second step.
  // Actually, wait - if we want to support "Activate Realm" button in step 2,
  // we should probably create it as DISABLED first, then update it?
  // OR, we create it as ENABLED if they click "Activate", and DISABLED if they click "Store".
  // But the secret is returned on CREATE. So we must create it first.
  
  // Let's stick to the plan:
  // Step 1: Input ID & Name. (Remove enabled toggle).
  // Step 2: Create (initially disabled). Show Secret.
  // Step 3: User clicks "Activate" -> Update to enabled=true.
  //         User clicks "Store only" -> Keep enabled=false.
  
  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setError("");
    setLoading(true);
    
    try {
      if (isEditing) {
        // Update existing
         await onSubmit({
          realm_id: parseInt(realmId, 10),
          name,
          enabled, // Use the state for editing
        });
        onClose();
      } else {
        // Create new - always create as DISABLED first to ensure we get secret
        // before activation decision in step 2
        const resp = await onSubmit({
          realm_id: parseInt(realmId, 10),
          name,
          enabled: false, // Create disabled initially
        });

        if (resp && 'realm_secret' in resp && resp.realm_secret) {
          setCreatedSecret(resp.realm_secret);
          setCreatedRealm({
              id: parseInt(realmId, 10),
              name: name
          });
          setStep("success");
          // Update local enabled state to false to match
          setEnabled(false); 
        } else {
          onClose();
        }
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Operation failed");
    } finally {
      setLoading(false);
    }
  }

  async function handleActivate() {
    if (!createdRealm) return;
    setLoading(true);
    try {
        await api.updateRealm(createdRealm.id, { enabled: true });
        if (onRefresh) onRefresh();
        onClose();
    } catch (err) {
        setError("Failed to activate realm");
    } finally {
        setLoading(false);
    }
  }

  function handleDownload() {
    if (!createdSecret || !createdRealm) return;
    const blob = new Blob([createdSecret], { type: "text/plain" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `realm-${createdRealm.id}-secret.txt`;
    a.click();
    URL.revokeObjectURL(url);
  }

  function handleCopy() {
    if (createdSecret) {
      navigator.clipboard.writeText(createdSecret);
    }
  }

  if (step === "success" && createdSecret && createdRealm) {
    return (
      <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
        <div className="w-full max-w-md rounded-xl bg-white p-6 shadow-xl">
          <div className="flex items-center justify-between mb-6">
            <h2 className="text-lg font-semibold text-gray-900 flex items-center gap-2">
              <CheckCircle className="h-5 w-5 text-green-500" />
              Realm Created
            </h2>
          </div>

          <div className="space-y-6">
            <div className="rounded-lg bg-amber-50 border border-amber-200 p-4">
              <div className="flex items-start gap-3">
                <AlertTriangle className="h-5 w-5 text-amber-600 shrink-0 mt-0.5" />
                <div className="space-y-1">
                  <p className="text-sm font-medium text-amber-900">Save this secret now</p>
                  <p className="text-xs text-amber-700">
                    This secret will only be shown once. It is required for actors to join this realm.
                  </p>
                </div>
              </div>
            </div>

            <div className="space-y-3">
                <div>
                    <label className="text-xs font-medium text-gray-500 uppercase">Realm ID</label>
                    <p className="text-sm font-mono text-gray-900">{createdRealm.id}</p>
                </div>
                <div>
                    <label className="text-xs font-medium text-gray-500 uppercase">Realm Name</label>
                    <p className="text-sm font-medium text-gray-900">{createdRealm.name}</p>
                </div>
                <div>
                    <label className="text-xs font-medium text-gray-500 uppercase">Realm Secret</label>
                    <div className="mt-1 flex items-center gap-2">
                        <code className="flex-1 rounded bg-gray-100 px-3 py-2 text-sm font-mono text-gray-800 border border-gray-200 break-all">
                            {createdSecret}
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
            </div>

            <div className="pt-4 border-t border-gray-100 flex flex-col gap-3">
                <button
                    onClick={handleActivate}
                    disabled={loading}
                    className="w-full rounded-lg bg-blue-600 px-4 py-2.5 text-sm font-medium text-white hover:bg-blue-700 transition-colors disabled:opacity-50"
                >
                    {loading ? "Activating..." : "I have saved the secret, Activate Realm"}
                </button>
                <button
                    onClick={onClose}
                    className="text-xs text-gray-500 hover:text-gray-700 hover:underline text-center"
                >
                    Store only (do not activate yet)
                </button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
      <div className="w-full max-w-md rounded-xl bg-white p-6 shadow-xl">
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-gray-900">
            {isEditing ? "Edit Realm" : "Create Realm"}
          </h2>
          <button
            onClick={onClose}
            className="rounded-lg p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 transition-colors"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="space-y-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">
              Realm ID
            </label>
            <input
              type="number"
              value={realmId}
              onChange={(e) => setRealmId(e.target.value)}
              className="w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500 disabled:bg-gray-50 disabled:text-gray-500 font-mono"
              required
              disabled={isEditing}
              min="1"
            />
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-700 mb-1">
              Name
            </label>
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="w-full rounded-lg border border-gray-300 px-3 py-2 text-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
              required
              autoFocus
            />
          </div>

          {isEditing && (
            <div>
              <label className="block text-sm font-medium text-gray-700 mb-1">
                Status
              </label>
              <div className="flex rounded-lg border border-gray-200 bg-gray-50 p-1">
                <button
                  type="button"
                  onClick={() => setEnabled(true)}
                  className={`flex-1 rounded-md py-1 text-sm font-medium transition-colors ${
                    enabled
                      ? "bg-white text-blue-600 shadow-sm"
                      : "text-gray-500 hover:text-gray-700"
                  }`}
                >
                  Enabled
                </button>
                <button
                  type="button"
                  onClick={() => setEnabled(false)}
                  className={`flex-1 rounded-md py-1 text-sm font-medium transition-colors ${
                    !enabled
                      ? "bg-white text-gray-900 shadow-sm"
                      : "text-gray-500 hover:text-gray-700"
                  }`}
                >
                  Stored only
                </button>
              </div>
              <p className="mt-1 text-xs text-gray-500">
                {enabled
                  ? "Realm is active and can be used by actors immediately."
                  : "Realm is saved but inactive. Actors cannot join until enabled."}
              </p>
            </div>
          )}

          {error && (
            <p className="text-sm text-red-600">{error}</p>
          )}

          <div className="flex justify-end gap-3 pt-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded-lg border border-gray-300 px-4 py-2 text-sm font-medium text-gray-700 hover:bg-gray-50 transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={loading}
              className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50 transition-colors"
            >
              {loading ? "Saving..." : isEditing ? "Update" : "Next"}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
