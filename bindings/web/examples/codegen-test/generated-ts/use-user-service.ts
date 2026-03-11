/**
 *  React Hook
 * : UserService
 *
 * ⚠️  
 */

import { useState, useEffect, useCallback } from 'react';
import { UserServiceActorRef } from './user-service.actor-ref';

/**
 * UserService React Hook
 */
export function useUserService(actorId: string) {
  const [actorRef] = useState(() => new UserServiceActorRef(actorId));
  const [isConnected, setIsConnected] = useState(false);

  useEffect(() => {
    // 
    const unlisten = actorRef.on('connection-state-changed', (state) => {
      setIsConnected(state === 'connected');
    });

    return () => {
      unlisten();
    };
  }, [actorRef]);

  /**
   * GetUser 
   */
  const getUser = useCallback(
    async (request: GetUserRequest) => {
      return actorRef.getUser(request);
    },
    [actorRef]
  );

  /**
   * CreateUser 
   */
  const createUser = useCallback(
    async (request: CreateUserRequest) => {
      return actorRef.createUser(request);
    },
    [actorRef]
  );

  /**
   * UpdateUser 
   */
  const updateUser = useCallback(
    async (request: UpdateUserRequest) => {
      return actorRef.updateUser(request);
    },
    [actorRef]
  );

  /**
   * DeleteUser 
   */
  const deleteUser = useCallback(
    async (request: DeleteUserRequest) => {
      return actorRef.deleteUser(request);
    },
    [actorRef]
  );


  return {
    actorRef,
    isConnected,
  };
}
