/**
 * useSubscription Hook
 */

import { useState, useEffect, useCallback } from 'react';
import { Actor } from '@actrium/actr-web';

export interface UseSubscriptionResult<T> {
  /** Array of received data */
  data: T[];

  /** Latest received data */
  latest: T | null;

  /** Error state */
  error: Error | null;

  /** Clear all data */
  clear: () => void;
}

/**
 * Hook to subscribe to a data stream
 *
 * @param client - Actor client instance
 * @param service - Service name
 * @param topic - Topic name
 * @param enabled - Whether the subscription is enabled (default: true)
 * @returns Subscription state and controls
 *
 * @example
 * ```tsx
 * function MetricsComponent({ client }) {
 *   const { data, latest, error } = useSubscription(
 *     client,
 *     'metrics-service',
 *     'cpu-usage'
 *   );
 *
 *   return (
 *     <div>
 *       {error && <div>Error: {error.message}</div>}
 *       {latest && <div>Current CPU: {latest.cpu}%</div>}
 *       <div>
 *         <h3>History:</h3>
 *         <ul>
 *           {data.map((item, i) => (
 *             <li key={i}>CPU: {item.cpu}%</li>
 *           ))}
 *         </ul>
 *       </div>
 *     </div>
 *   );
 * }
 * ```
 */
export function useSubscription<T>(
  client: Actor | null,
  service: string,
  topic: string,
  enabled: boolean = true
): UseSubscriptionResult<T> {
  const [data, setData] = useState<T[]>([]);
  const [latest, setLatest] = useState<T | null>(null);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    if (!client || !enabled) {
      return;
    }

    const currentClient = client;
    let unsubscribe: (() => void) | null = null;

    async function subscribe() {
      try {
        setError(null);

        unsubscribe = await currentClient.subscribe<T>(`${service}.${topic}`, (newData: T) => {
          setLatest(newData);
          setData((prev) => [...prev, newData]);
        });
      } catch (err) {
        setError(err as Error);
      }
    }

    subscribe();

    return () => {
      if (unsubscribe) {
        unsubscribe();
      }
    };
  }, [client, service, topic, enabled]);

  const clear = useCallback(() => {
    setData([]);
    setLatest(null);
  }, []);

  return {
    data,
    latest,
    error,
    clear,
  };
}
