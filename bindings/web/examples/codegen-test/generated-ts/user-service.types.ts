/**
 * 自动生成的类型定义
 * 服务: UserService
 * 包: example.user.v1
 *
 * ⚠️  请勿手动编辑此文件
 */

/**
 * GetUserRequest 消息
 */
export interface GetUserRequest {
  user_id: string;
}

/**
 * GetUserResponse 消息
 */
export interface GetUserResponse {
  user: User;
}

/**
 * CreateUserRequest 消息
 */
export interface CreateUserRequest {
  name: string;
  email: string;
  age?: number;
  tags: string[];
}

/**
 * CreateUserResponse 消息
 */
export interface CreateUserResponse {
  user: User;
}

/**
 * UpdateUserRequest 消息
 */
export interface UpdateUserRequest {
  user_id: string;
  name?: string;
  email?: string;
  age?: number;
}

/**
 * UpdateUserResponse 消息
 */
export interface UpdateUserResponse {
  user: User;
  updated: boolean;
}

/**
 * DeleteUserRequest 消息
 */
export interface DeleteUserRequest {
  user_id: string;
}

/**
 * DeleteUserResponse 消息
 */
export interface DeleteUserResponse {
  deleted: boolean;
}

/**
 * ListUsersRequest 消息
 */
export interface ListUsersRequest {
  page: number;
  page_size: number;
  filter?: string;
}

/**
 * WatchUsersRequest 消息
 */
export interface WatchUsersRequest {
  user_ids: string[];
}

/**
 * User 消息
 */
export interface User {
  id: string;
  name: string;
  email: string;
  age: number;
  tags: string[];
  created_at: number;
  updated_at: number;
  is_active: boolean;
}

/**
 * UserEvent 消息
 */
export interface UserEvent {
  event_type: string;
  user: User;
  timestamp: number;
}

