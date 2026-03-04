/**
 * useActorClient Hook
 */

import { useState, useEffect, useCallback } from 'react';
import { ActorClient, ActorClientConfig, ConnectionState } from '@actr/web';

export interface UseActorClientResult {
  /** Actor client instance */
  client: ActorClient | null;

  /** Connection state */
  connectionState: ConnectionState;

  /** Loading state */
  loading: boolean;

  /** Error state */
  error: Error | null;

  /** Reconnect function */
  reconnect: () => Promise<void>;
}

/**
 * Hook to create and manage an Actor client
 *
 * @param config - Actor client configuration
 * @returns Actor client state and controls
 *
 * @example
 * ```tsx
 * function App() {
 *   const { client, loading, error } = useActorClient({
 *     signalingUrl: 'wss://signal.example.com',
 *     realm: 'demo',
 *   });
 *
 *   if (loading) return <div>Connecting...</div>;
 *   if (error) return <div>Error: {error.message}</div>;
 *
 *   return <div>Connected!</div>;
 * }
 * ```
 */
export function useActorClient(config: ActorClientConfig): UseActorClientResult {
  const [client, setClient] = useState<ActorClient | null>(null);
  const [connectionState, setConnectionState] = useState<ConnectionState>(
    'disconnected' as ConnectionState
  );
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  // Initialize client
  useEffect(() => {
    let mounted = true;
    let currentClient: ActorClient | null = null;

    async function initClient() {
      try {
        setLoading(true);
        setError(null);

        const newClient = await ActorClient.create(config);

        if (mounted) {
          currentClient = newClient;
          setClient(newClient);
          setConnectionState(newClient.getConnectionState());

          // Listen to state changes
          newClient.on('stateChange', (state: ConnectionState) => {
            if (mounted) {
              setConnectionState(state);
            }
          });
        }
      } catch (err) {
        if (mounted) {
          setError(err as Error);
        }
      } finally {
        if (mounted) {
          setLoading(false);
        }
      }
    }

    initClient();

    // Cleanup
    return () => {
      mounted = false;
      if (currentClient) {
        currentClient.close().catch(console.error);
      }
    };
  }, [config.signalingUrl, config.realm]); // Only recreate if these change

  // Reconnect function
  const reconnect = useCallback(async () => {
    if (client) {
      await client.close();
    }

    setLoading(true);
    setError(null);

    try {
      const newClient = await ActorClient.create(config);
      setClient(newClient);
      setConnectionState(newClient.getConnectionState());
    } catch (err) {
      setError(err as Error);
    } finally {
      setLoading(false);
    }
  }, [client, config]);

  return {
    client,
    connectionState,
    loading,
    error,
    reconnect,
  };
}
