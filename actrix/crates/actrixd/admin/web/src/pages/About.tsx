export function AboutPage() {
  return (
    <div className="mx-auto max-w-4xl space-y-4 p-6">
      <h1 className="text-2xl font-bold text-gray-900">About Actrix</h1>
      <p className="text-sm text-gray-600 leading-6">
        Actrix is an open-source, multi-tenant real-time communication stack with Signer, AIS, Signaling, STUN/TURN, and control-plane tooling.
      </p>
      <div className="space-y-3 text-sm text-gray-700 leading-6">
        <p>License: Apache 2.0</p>
        <p>Repository: github.com/Actrium/actrix</p>
        <p>Version: (see Releases)</p>
      </div>
      <div className="border-t border-gray-100 pt-4 text-xs text-gray-400">
        Some icons from <a href="https://lucide.dev/" target="_blank" rel="noopener noreferrer" className="underline hover:text-gray-500">Lucide</a>, thanks!
      </div>
    </div>
  );
}
