/**
 * 自动生成的 React Hook
 * 服务: UserService
 *
 * ⚠️  请勿手动编辑此文件
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
    // 监听连接状态
    const unlisten = actorRef.on('connection-state-changed', (state) => {
      setIsConnected(state === 'connected');
    });

    return () => {
      unlisten();
    };
  }, [actorRef]);

  /**
   * GetUser 方法的便捷调用
   */
  const getUser = useCallback(
    async (request: GetUserRequest) => {
      return actorRef.getUser(request);
    },
    [actorRef]
  );

  /**
   * CreateUser 方法的便捷调用
   */
  const createUser = useCallback(
    async (request: CreateUserRequest) => {
      return actorRef.createUser(request);
    },
    [actorRef]
  );

  /**
   * UpdateUser 方法的便捷调用
   */
  const updateUser = useCallback(
    async (request: UpdateUserRequest) => {
      return actorRef.updateUser(request);
    },
    [actorRef]
  );

  /**
   * DeleteUser 方法的便捷调用
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
