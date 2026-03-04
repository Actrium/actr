/**
 * 自动生成的 ActorRef 包装
 * 服务: UserService
 *
 * ⚠️  请勿手动编辑此文件
 */

import { ActorRef } from '@actr/web';
import type { DeleteUserRequest, CreateUserRequest, ListUsersRequest, UpdateUserResponse, WatchUsersRequest, GetUserResponse, GetUserRequest, DeleteUserResponse, CreateUserResponse, UpdateUserRequest, User, UserEvent } from './user-service.types';

/**
 * UserService Actor 引用
 */
export class UserServiceActorRef extends ActorRef {
  /**
   * 创建新的 ActorRef 实例
   */
  constructor(actorId: string) {
    super(actorId);
  }

  /**
   * GetUser 方法
   */
  async getUser(request: GetUserRequest): Promise<GetUserResponse> {
    return this.call('UserService', 'GetUser', request);
  }

  /**
   * CreateUser 方法
   */
  async createUser(request: CreateUserRequest): Promise<CreateUserResponse> {
    return this.call('UserService', 'CreateUser', request);
  }

  /**
   * UpdateUser 方法
   */
  async updateUser(request: UpdateUserRequest): Promise<UpdateUserResponse> {
    return this.call('UserService', 'UpdateUser', request);
  }

  /**
   * DeleteUser 方法
   */
  async deleteUser(request: DeleteUserRequest): Promise<DeleteUserResponse> {
    return this.call('UserService', 'DeleteUser', request);
  }

  /**
   * ListUsers 方法（流式）
   */
  subscribeListUsers(callback: (data: User) => void): () => void {
    return this.subscribe('UserService:ListUsers', callback);
  }

  /**
   * WatchUsers 方法（流式）
   */
  subscribeWatchUsers(callback: (data: UserEvent) => void): () => void {
    return this.subscribe('UserService:WatchUsers', callback);
  }

}
