/**
 *  ActorRef 
 * : UserService
 *
 * ⚠️  
 */

import { ActorRef } from '@actr/web';
import type { DeleteUserRequest, CreateUserRequest, ListUsersRequest, UpdateUserResponse, WatchUsersRequest, GetUserResponse, GetUserRequest, DeleteUserResponse, CreateUserResponse, UpdateUserRequest, User, UserEvent } from './user-service.types';

/**
 * UserService Actor 
 */
export class UserServiceActorRef extends ActorRef {
  /**
   *  ActorRef 
   */
  constructor(actorId: string) {
    super(actorId);
  }

  /**
   * GetUser 
   */
  async getUser(request: GetUserRequest): Promise<GetUserResponse> {
    return this.call('UserService', 'GetUser', request);
  }

  /**
   * CreateUser 
   */
  async createUser(request: CreateUserRequest): Promise<CreateUserResponse> {
    return this.call('UserService', 'CreateUser', request);
  }

  /**
   * UpdateUser 
   */
  async updateUser(request: UpdateUserRequest): Promise<UpdateUserResponse> {
    return this.call('UserService', 'UpdateUser', request);
  }

  /**
   * DeleteUser 
   */
  async deleteUser(request: DeleteUserRequest): Promise<DeleteUserResponse> {
    return this.call('UserService', 'DeleteUser', request);
  }

  /**
   * ListUsers （）
   */
  subscribeListUsers(callback: (data: User) => void): () => void {
    return this.subscribe('UserService:ListUsers', callback);
  }

  /**
   * WatchUsers （）
   */
  subscribeWatchUsers(callback: (data: UserEvent) => void): () => void {
    return this.subscribe('UserService:WatchUsers', callback);
  }

}
