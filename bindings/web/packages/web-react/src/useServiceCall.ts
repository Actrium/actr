/**
 * useServiceCall Hook
 */

import { useState, useCallback } from 'react';
import { ActorClient, RpcOptions } from '@actr/web';

export interface UseServiceCallResult<TRequest, TResponse> {
  /** Call the service method */
  call: (request: TRequest, options?: RpcOptions) => Promise<TResponse>;

  /** Response data */
  data: TResponse | null;

  /** Loading state */
  loading: boolean;

  /** Error state */
  error: Error | null;

  /** Reset state */
  reset: () => void;
}

/**
 * Hook to call a service method
 *
 * @param client - Actor client instance
 * @param service - Service name
 * @param method - Method name
 * @returns Service call state and controls
 *
 * @example
 * ```tsx
 * function EchoComponent({ client }) {
 *   const { call, data, loading, error } = useServiceCall(
 *     client,
 *     'echo-service',
 *     'sendEcho'
 *   );
 *
 *   const handleClick = async () => {
 *     await call({ message: 'Hello!' });
 *   };
 *
 *   return (
 *     <div>
 *       <button onClick={handleClick} disabled={loading}>
 *         Send Echo
 *       </button>
 *       {loading && <div>Loading...</div>}
 *       {error && <div>Error: {error.message}</div>}
 *       {data && <div>Reply: {data.reply}</div>}
 *     </div>
 *   );
 * }
 * ```
 */
export function useServiceCall<TRequest, TResponse>(
  client: ActorClient | null,
  service: string,
  method: string
): UseServiceCallResult<TRequest, TResponse> {
  const [data, setData] = useState<TResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Error | null>(null);

  const call = useCallback(
    async (request: TRequest, options?: RpcOptions): Promise<TResponse> => {
      if (!client) {
        throw new Error('Client not initialized');
      }

      setLoading(true);
      setError(null);

      try {
        const response = await client.call<TRequest, TResponse>(service, method, request, options);

        setData(response);
        return response;
      } catch (err) {
        const error = err as Error;
        setError(error);
        throw error;
      } finally {
        setLoading(false);
      }
    },
    [client, service, method]
  );

  const reset = useCallback(() => {
    setData(null);
    setError(null);
    setLoading(false);
  }, []);

  return {
    call,
    data,
    loading,
    error,
    reset,
  };
}
